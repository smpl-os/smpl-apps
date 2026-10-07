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

void FakeKdenliveClient::control(const QString &name, double delta, const QVariantMap &options)
{
    record(QStringLiteral("control %1 %2%3").arg(name).arg(delta).arg(compact(options)));
    controlDeltas << delta;
    if (m_autoAck) {
        QTimer::singleShot(0, this, [this, name] { Q_EMIT controlAcked(name, {}); });
    } else {
        m_unacked << name;
    }
}

void FakeKdenliveClient::ackAll()
{
    const QStringList pending = m_unacked;
    m_unacked.clear();
    for (const auto &n : pending) {
        Q_EMIT controlAcked(n, {});
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

void FakeKdenliveClient::setAvailable(bool on)
{
    if (m_available != on) {
        m_available = on;
        Q_EMIT availabilityChanged(on);
    }
}

void FakeKdenliveClient::setContext(const QVariantMap &ctx)
{
    m_context = ctx;
    Q_EMIT contextChanged(ctx);
}

} // namespace cs
