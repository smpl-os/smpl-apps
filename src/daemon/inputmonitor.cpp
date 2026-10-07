// SPDX-License-Identifier: GPL-2.0-or-later
#include "inputmonitor.h"
#include "settingsservice.h"

#include <QDBusConnectionInterface>
#include <QDBusServiceWatcher>
#include <QJsonDocument>
#include <QJsonObject>

namespace cs {

bool InputMonitor::attach(const QDBusConnection &bus)
{
    QDBusConnection b(bus);
    const QString service = QLatin1String(SettingsService::kService);
    if (!b.interface() || !b.interface()->isServiceRegistered(service)) {
        return false;
    }
    auto *watch = new QDBusServiceWatcher(service, b, QDBusServiceWatcher::WatchForUnregistration, this);
    connect(watch, &QDBusServiceWatcher::serviceUnregistered, this, &InputMonitor::daemonGone);
    return b.connect(service, QLatin1String(SettingsService::kPath), QLatin1String(SettingsService::kInterface), QStringLiteral("InputEvent"), this,
                     SLOT(onInputEvent(QString, QString, int)));
}

void InputMonitor::onInputEvent(const QString &slot, const QString &event, int delta)
{
    Q_EMIT input(slot, event, delta);
}

QString InputMonitor::jsonLine(const QString &slot, const QString &event, int delta, qint64 ms)
{
    return QString::fromUtf8(QJsonDocument(QJsonObject{{QStringLiteral("slot"), slot},
                                                       {QStringLiteral("event"), event},
                                                       {QStringLiteral("delta"), delta},
                                                       {QStringLiteral("ms"), ms}})
                                 .toJson(QJsonDocument::Compact));
}

QString InputMonitor::textLine(const QString &slot, const QString &event, int delta)
{
    return delta ? QStringLiteral("%1 %2 %3").arg(slot, event).arg(delta) : QStringLiteral("%1 %2").arg(slot, event);
}

} // namespace cs
