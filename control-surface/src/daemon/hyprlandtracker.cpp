// SPDX-License-Identifier: GPL-2.0-or-later
#include "windowtracker.h"

#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QTimer>

#include <memory>

namespace cs {

HyprlandTracker::HyprlandTracker(const QString &socketDir, QObject *parent)
    : WindowTracker(parent)
    , m_dir(socketDir)
    , m_fixedDir(!socketDir.isEmpty())
{
    m_reconnect = new QTimer(this);
    m_reconnect->setSingleShot(true);
    m_reconnect->setInterval(2000);
    connect(m_reconnect, &QTimer::timeout, this, &HyprlandTracker::connectEvents);
}

QString HyprlandTracker::resolveDir() const
{
    if (m_fixedDir) {
        return m_dir;
    }
    const QString runtime = qEnvironmentVariable("XDG_RUNTIME_DIR");
    const QString sig = qEnvironmentVariable("HYPRLAND_INSTANCE_SIGNATURE");
    const QString candidate = runtime + QStringLiteral("/hypr/") + sig;
    if (!sig.isEmpty() && QFileInfo::exists(candidate + QStringLiteral("/.socket2.sock"))) {
        return candidate;
    }
    // Hyprland restarted with a new signature: take the newest live instance.
    QDir hypr(runtime + QStringLiteral("/hypr"));
    QString best;
    QDateTime bestTime;
    for (const auto &e : hypr.entryInfoList(QDir::Dirs | QDir::NoDotAndDotDot)) {
        const QFileInfo sock(e.filePath() + QStringLiteral("/.socket2.sock"));
        if (sock.exists() && (best.isEmpty() || sock.lastModified() > bestTime)) {
            best = e.filePath();
            bestTime = sock.lastModified();
        }
    }
    return best.isEmpty() ? candidate : best;
}

void HyprlandTracker::start()
{
    connectEvents();
}

void HyprlandTracker::connectEvents()
{
    m_dir = resolveDir();
    if (m_events) {
        m_events->disconnect(this);
        m_events->abort();
        m_events->deleteLater();
    }
    m_events = new QLocalSocket(this);
    connect(m_events, &QLocalSocket::connected, this, [this] {
        Q_EMIT message(QStringLiteral("hyprland events connected (%1)").arg(m_dir));
        queryActive();
    });
    connect(m_events, &QLocalSocket::readyRead, this, [this] {
        m_buffer += m_events->readAll();
        qsizetype nl;
        while ((nl = m_buffer.indexOf('\n')) >= 0) {
            const QString line = QString::fromUtf8(m_buffer.left(nl));
            m_buffer.remove(0, nl + 1);
            handleEventLine(line);
        }
    });
    connect(m_events, &QLocalSocket::disconnected, this, [this] {
        Q_EMIT message(QStringLiteral("hyprland events disconnected, retrying"));
        m_reconnect->start();
    });
    connect(m_events, &QLocalSocket::errorOccurred, this, [this](QLocalSocket::LocalSocketError) {
        if (!m_reconnect->isActive()) {
            m_reconnect->start();
        }
    });
    m_events->connectToServer(m_dir + QStringLiteral("/.socket2.sock"));
}

void HyprlandTracker::handleEventLine(const QString &line)
{
    const int sep = line.indexOf(QLatin1String(">>"));
    if (sep < 0) {
        return;
    }
    const QString ev = line.left(sep);
    const QString data = line.mid(sep + 2);
    if (ev == QLatin1String("activewindow")) {
        // class,title (title may contain commas); provisional until the query answers
        const int comma = data.indexOf(QLatin1Char(','));
        WindowInfo w;  // pid/address unknown until the query answers
        w.cls = comma < 0 ? data : data.left(comma);
        w.title = comma < 0 ? QString() : data.mid(comma + 1);
        if (w.cls != current().cls || w.title != current().title) {
            setCurrent(w);
        }
        queryActive();
    } else if (ev == QLatin1String("activewindowv2") || ev == QLatin1String("closewindow") || ev == QLatin1String("configreloaded")) {
        queryActive();
    } else if (ev == QLatin1String("windowtitlev2")) {
        const int comma = data.indexOf(QLatin1Char(','));
        const QString addr = QStringLiteral("0x") + data.left(comma);
        if (comma > 0 && addr == current().address) {
            WindowInfo w = current();
            w.title = data.mid(comma + 1);
            setCurrent(w);
        }
    }
}

void HyprlandTracker::queryActive()
{
    if (m_queryInFlight) {
        m_queryDirty = true;
        return;
    }
    m_queryInFlight = true;
    m_queryDirty = false;
    auto *s = new QLocalSocket(this);
    auto buf = std::make_shared<QByteArray>();
    connect(s, &QLocalSocket::connected, s, [s] { s->write("j/activewindow"); });
    connect(s, &QLocalSocket::readyRead, s, [s, buf] { buf->append(s->readAll()); });
    auto done = [this, s] {
        s->disconnect();
        s->deleteLater();
        m_queryInFlight = false;
        if (m_queryDirty) {
            queryActive();
        }
    };
    connect(s, &QLocalSocket::disconnected, this, [this, s, buf, done] {
        buf->append(s->readAll());
        const QJsonDocument doc = QJsonDocument::fromJson(*buf);
        // A newer focus event arrived while this query ran: its answer may describe
        // the previous window, so skip it and let the follow-up query decide.
        if (doc.isObject() && !m_queryDirty) {  // "{}" means no focused window
            const QJsonObject o = doc.object();
            WindowInfo w;
            w.cls = o.value(QStringLiteral("class")).toString();
            w.title = o.value(QStringLiteral("title")).toString();
            w.pid = o.value(QStringLiteral("pid")).toInteger();
            w.address = o.value(QStringLiteral("address")).toString();
            setCurrent(w);
        }
        done();
    });
    connect(s, &QLocalSocket::errorOccurred, this, [done](QLocalSocket::LocalSocketError e) {
        if (e != QLocalSocket::PeerClosedError) {
            done();
        }
    });
    s->connectToServer(m_dir + QStringLiteral("/.socket.sock"));
}

} // namespace cs
