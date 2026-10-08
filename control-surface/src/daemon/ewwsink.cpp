// SPDX-License-Identifier: GPL-2.0-or-later
#include "ewwsink.h"
#include "cheatsheet.h"

#include <QElapsedTimer>
#include <algorithm>
#include <QJsonDocument>
#include <QTimer>

namespace cs {

namespace {
QByteArray compact(const QJsonObject &o)
{
    return QJsonDocument(o).toJson(QJsonDocument::Compact);
}

QString lastLine(const QString &output)
{
    const QStringList lines = output.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
    QString l = lines.isEmpty() ? QString() : lines.last().trimmed();
    const int arrow = l.indexOf(QLatin1String(" > "));  // eww: "<time> ERROR <module> > <text>"
    if (arrow >= 0) {
        l = l.mid(arrow + 3);
    }
    return l.left(200);
}
} // namespace

EwwSink::EwwSink(Cheatsheet *sheet, QObject *parent)
    : QObject(parent)
    , m_sheet(sheet)
    , m_timeout(new QTimer(this))
{
    m_timeout->setSingleShot(true);
    m_timeout->setInterval(kCallTimeoutMs);
    connect(m_timeout, &QTimer::timeout, this, [this] {
        if (m_proc) {
            m_proc->kill();  // finished() follows as a crash
        }
    });
    connect(sheet, &Cheatsheet::changed, this, [this](const QJsonObject &c) { push(c, true); });
    connect(sheet, &Cheatsheet::visibilityChanged, this, [this](bool on) {
        if (!on) {
            push(m_sheet->content(), false);
        }
    });
    connect(sheet, &Cheatsheet::hideForced, this, &EwwSink::forceHide);
}

bool EwwSink::pending(int kind) const
{
    if (m_proc && m_running.kind == kind) {
        return true;
    }
    return std::any_of(m_steps.cbegin(), m_steps.cend(), [kind](const Step &s) { return s.kind == kind; });
}

void EwwSink::forceHide()
{
    if (!m_opts.enabled || !m_sheet) {
        return;
    }
    // Whatever eww was told (a window left open, a variable out of step):
    // close the window and send the hidden state, unless the hide that just
    // happened already has them on their way.
    m_forceClose = !m_opts.window.isEmpty() && !pending(Step::Close);
    m_forceUpdate = !pending(Step::Update);
    push(m_sheet->content(), false);
}

EwwSink::~EwwSink()
{
    if (m_proc) {
        QProcess *p = m_proc;
        m_proc = nullptr;
        p->disconnect(this);
        p->kill();
        p->waitForFinished(500);
    }
}

QStringList EwwSink::baseArgs() const
{
    return m_opts.configDir.isEmpty() ? QStringList{} : QStringList{QStringLiteral("--config"), m_opts.configDir};
}

QByteArray EwwSink::hidden(const QByteArray &json)
{
    QJsonObject o = QJsonDocument::fromJson(json).object();
    o.insert(QStringLiteral("visible"), false);
    return compact(o);
}

void EwwSink::setOptions(const EwwHook &o)
{
    if (o == m_opts) {
        return;
    }
    if (m_opts.enabled) {
        // Take down what the old settings put on screen.
        const QStringList base = baseArgs();
        if (!m_opts.window.isEmpty() && m_windowOpen) {
            m_steps.append(Step{Step::Close, m_opts.binary, base + QStringList{QStringLiteral("close"), m_opts.window}});
        }
        if (m_sentVisible) {
            m_steps.append(Step{Step::Update, m_opts.binary,
                                base + QStringList{QStringLiteral("update"), m_opts.variable + QLatin1Char('=') + QString::fromUtf8(hidden(m_sentJson))}});
        }
    }
    m_opts = o;
    m_sentJson.clear();
    m_sentVisible = false;
    m_lastUpdateOk = true;
    if (m_opts.enabled && m_sheet) {
        const QJsonObject c = m_sheet->content();
        m_wantJson = compact(c);
        m_wantAnchor = EwwHook::anchorFor(c.value(QStringLiteral("options")).toObject().value(QStringLiteral("position")).toString());
        m_wantVisible = m_sheet->isVisible();
        m_hasWant = true;
    }
    // Whether the window is open is unknown (a crash may have left it): a
    // hidden sheet closes it once.
    m_windowOpen = m_opts.enabled && !m_opts.window.isEmpty() && !m_wantVisible;
    if (!m_proc) {
        if (m_steps.isEmpty()) {
            plan();
        }
        startNext();
    }
}

void EwwSink::push(const QJsonObject &content, bool visible)
{
    m_wantJson = compact(content);
    m_wantAnchor = EwwHook::anchorFor(content.value(QStringLiteral("options")).toObject().value(QStringLiteral("position")).toString());
    m_wantVisible = visible;
    m_hasWant = true;
    if (m_opts.enabled && !isBusy()) {
        plan();
        startNext();
    }
}

void EwwSink::plan()
{
    if (!m_opts.enabled || !m_hasWant) {
        return;
    }
    const QStringList base = baseArgs();
    const bool changed = m_wantJson != m_sentJson;
    auto update = [&] {
        m_sentJson = m_wantJson;
        m_sentVisible = m_wantVisible;
        if (m_wantJson.size() > kMaxJsonBytes) {
            ++m_failures;
            m_lastUpdateOk = false;
            m_lastError = QStringLiteral("content too large for one eww call (%1 bytes)").arg(m_wantJson.size());
            Q_EMIT message(QStringLiteral("eww: %1").arg(m_lastError));
            return false;
        }
        m_steps.append(Step{Step::Update, m_opts.binary,
                            base + QStringList{QStringLiteral("update"), m_opts.variable + QLatin1Char('=') + QString::fromUtf8(m_wantJson)}});
        return true;
    };
    if (m_wantVisible) {
        m_forceClose = m_forceUpdate = false;
        if (!m_opts.window.isEmpty() && m_windowOpen && m_openAnchor != m_wantAnchor) {
            // eww anchors a window when it opens: a new position needs a reopen.
            m_steps.append(Step{Step::Close, m_opts.binary, base + QStringList{QStringLiteral("close"), m_opts.window}});
            m_windowOpen = false;
        }
        const bool queued = changed && update();
        if (!m_opts.window.isEmpty() && !m_windowOpen && (queued || (!changed && m_lastUpdateOk))) {
            m_steps.append(Step{Step::Open, m_opts.binary,
                                base + QStringList{QStringLiteral("open"), m_opts.window, QStringLiteral("--anchor"), m_wantAnchor}});
            m_windowOpen = true;
            m_openAnchor = m_wantAnchor;
        }
    } else {
        if (!m_opts.window.isEmpty() && (m_windowOpen || m_forceClose)) {
            m_steps.append(Step{Step::Close, m_opts.binary, base + QStringList{QStringLiteral("close"), m_opts.window}});
            m_windowOpen = false;
        }
        if (changed || m_forceUpdate) {
            update();
        }
        m_forceClose = m_forceUpdate = false;
    }
}

void EwwSink::startNext()
{
    if (m_proc || m_steps.isEmpty()) {
        return;
    }
    m_running = m_steps.takeFirst();
    auto *p = new QProcess(this);
    m_proc = p;
    p->setProcessChannelMode(QProcess::MergedChannels);
    p->setStandardInputFile(QProcess::nullDevice());
    connect(p, &QProcess::finished, this, [this, p](int code, QProcess::ExitStatus st) {
        const QString out = lastLine(QString::fromUtf8(p->readAll()));
        const bool ok = st == QProcess::NormalExit && code == 0;
        done(p, ok, !out.isEmpty() ? out : st == QProcess::NormalExit ? QStringLiteral("exit code %1").arg(code) : QStringLiteral("no answer, stopped"));
    });
    connect(p, &QProcess::errorOccurred, this, [this, p](QProcess::ProcessError e) {
        if (e == QProcess::FailedToStart) {
            done(p, false, QStringLiteral("cannot run '%1' (%2)").arg(m_running.program, p->errorString()));
        }
    });
    ++m_calls;
    m_timeout->start();
    p->start(m_running.program, m_running.args);
}

void EwwSink::done(QProcess *p, bool ok, const QString &output)
{
    if (p != m_proc) {
        return;
    }
    m_timeout->stop();
    m_proc = nullptr;
    p->disconnect(this);
    p->deleteLater();
    const Step::Kind kind = m_running.kind;
    if (kind == Step::Update) {
        m_lastUpdateOk = ok;
        if (!ok) {
            // eww is not answering: an `open` now would start a daemon.
            if (m_steps.removeIf([](const Step &s) { return s.kind == Step::Open; }) > 0) {
                m_windowOpen = false;
            }
        }
    }
    static const char *verbs[] = {"update", "open", "close"};
    if (!ok && kind != Step::Close) {  // closing a window that is not open is fine
        ++m_failures;
        m_lastError = QStringLiteral("%1: %2").arg(QLatin1String(verbs[kind]), output);
        if (!m_failing) {
            m_failing = true;
            Q_EMIT message(QStringLiteral("eww: %1 (reported once until it works again)").arg(m_lastError));
        }
    } else if (ok && m_failing && kind != Step::Close) {
        m_failing = false;
        Q_EMIT message(QStringLiteral("eww: working again"));
    }
    if (m_steps.isEmpty()) {
        plan();  // the latest wanted state
    }
    startNext();
    if (!isBusy()) {
        Q_EMIT idle();
    }
}

void EwwSink::finish(int timeoutMs)
{
    if (m_opts.enabled && m_hasWant && m_wantVisible) {
        push(QJsonDocument::fromJson(hidden(m_wantJson)).object(), false);
    }
    QElapsedTimer t;
    t.start();
    while (isBusy()) {
        const int left = timeoutMs - int(t.elapsed());
        if (left <= 0) {
            break;
        }
        if (!m_proc) {
            startNext();
            continue;
        }
        QProcess *p = m_proc;
        if (!p->waitForFinished(left) && m_proc == p) {
            break;  // eww hangs; give up
        }
    }
    m_timeout->stop();
    m_steps.clear();
    if (m_proc) {
        QProcess *p = m_proc;
        m_proc = nullptr;
        p->disconnect(this);
        p->kill();
        p->waitForFinished(500);
        delete p;
    }
}

QJsonObject EwwSink::status() const
{
    return QJsonObject{{QStringLiteral("enabled"), m_opts.enabled},
                       {QStringLiteral("variable"), m_opts.variable},
                       {QStringLiteral("window"), m_opts.window},
                       {QStringLiteral("config"), m_opts.configDir},
                       {QStringLiteral("calls"), m_calls},
                       {QStringLiteral("failures"), m_failures},
                       {QStringLiteral("lastError"), m_lastError}};
}

} // namespace cs
