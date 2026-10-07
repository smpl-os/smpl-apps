// SPDX-License-Identifier: GPL-2.0-or-later
#include "kdenliveclient.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>
#include <cstdio>

namespace cs {

FakeKdenliveClient::FakeKdenliveClient(bool print, QObject *parent)
    : KdenliveClient(parent)
    , m_print(print)
{
}

void FakeKdenliveClient::record(const QString &line)
{
    if (calls.size() >= 10000) {
        calls.removeFirst();  // long --dry-run sessions
    }
    calls << line;
    if (m_print) {
        std::printf("  -> kdenlive %s\n", qPrintable(line));
        std::fflush(stdout);
    }
}

static QString compact(const QVariantMap &m)
{
    if (m.isEmpty()) {
        return {};
    }
    return QLatin1Char(' ') + QString::fromUtf8(QJsonDocument(QJsonObject::fromVariantMap(m)).toJson(QJsonDocument::Compact));
}

void FakeKdenliveClient::attachToPid(qint64 pid)
{
    m_pid = pid;
}

void FakeKdenliveClient::triggerAction(const QString &id)
{
    record(QStringLiteral("action %1").arg(id));
}

bool FakeKdenliveClient::control(const QString &key, const QString &name, double delta, const QVariantMap &options)
{
    // Bookkeeping options are omitted from the printed form to keep simulate readable.
    QVariantMap shown = options;
    shown.remove(QStringLiteral("gesture"));
    shown.remove(QStringLiteral("target"));
    const QString phase = shown.take(QStringLiteral("phase")).toString();
    record(QStringLiteral("control %1 %2%3%4")
               .arg(name)
               .arg(delta)
               .arg(compact(shown), phase.isEmpty() || phase == QLatin1String("update") ? QString() : QStringLiteral(" (%1)").arg(phase)));
    controlDeltas << delta;
    controlOptions << options;
    controlKeys << key;
    if (m_autoAck) {
        QTimer::singleShot(0, this, [this, key] { Q_EMIT controlAcked(key, {{QStringLiteral("ok"), true}}); });
    } else {
        m_unacked << key;
    }
    return true;
}

void FakeKdenliveClient::ackAll(const QVariantMap &outcome)
{
    const QStringList pending = m_unacked;
    m_unacked.clear();
    for (const auto &k : pending) {
        Q_EMIT controlAcked(k, outcome);
    }
}

void FakeKdenliveClient::invoke(const QString &command, const QVariantMap &args)
{
    record(QStringLiteral("invoke %1%2").arg(command, compact(args)));
}

void FakeKdenliveClient::notify(const QString &text)
{
    record(QStringLiteral("notify %1").arg(text));
}

void FakeKdenliveClient::setState(State s)
{
    if (m_state != s) {
        m_state = s;
        Q_EMIT stateChanged(s);
    }
}

void FakeKdenliveClient::setContext(const QVariantMap &ctx)
{
    const quint64 before = epoch();
    m_context = ctx;
    Q_EMIT contextChanged(ctx);
    if (epoch() != before) {
        Q_EMIT epochChanged(epoch());
    }
}

void FakeKdenliveClient::setCapabilities(const QStringList &controls, const QStringList &actions, const QStringList &commands)
{
    m_restricted = true;
    m_restrictedActions = true;
    m_controls = QSet<QString>(controls.begin(), controls.end());
    m_actions = QSet<QString>(actions.begin(), actions.end());
    m_commands = QSet<QString>(commands.begin(), commands.end());
}

void FakeKdenliveClient::setControlCapabilities(const QStringList &controls, const QStringList &commands)
{
    m_restricted = true;
    m_controls = QSet<QString>(controls.begin(), controls.end());
    m_commands = QSet<QString>(commands.begin(), commands.end());
}

} // namespace cs
