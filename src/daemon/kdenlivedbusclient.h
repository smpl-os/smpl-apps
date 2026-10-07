// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "kdenliveclient.h"

#include <QDBusConnection>
#include <QHash>

class QDBusServiceWatcher;

namespace cs {

// Talks to org.kde.kdenlive.ControlSurface1 on org.kde.kdenlive-<pid> (session
// bus) or on a peer-to-peer connection (tests, optional direct socket).
class KdenliveDBusClient : public KdenliveClient
{
    Q_OBJECT
public:
    explicit KdenliveDBusClient(const QDBusConnection &connection, QObject *parent = nullptr);
    ~KdenliveDBusClient() override;

    // Fixed target, ignoring window pids (mock testing). Empty + peer connection = the peer.
    void setServiceOverride(const QString &service);
    void attachToService(const QString &service);

    bool isAvailable() const override { return m_available; }
    QVariantMap context() const override { return m_context; }
    void attachToPid(qint64 pid) override;
    qint64 attachedPid() const override { return m_pid; }
    void triggerAction(const QString &id) override;
    void control(const QString &name, double delta, const QVariantMap &options) override;
    void invoke(const QString &command, const QVariantMap &args) override;
    void notify(const QString &text) override;

    static QVariant normalize(const QVariant &v);  // QDBusArgument trees -> plain QVariant

private Q_SLOTS:
    void onContextChanged(const QVariantMap &context);
    void onControlAck(uint seq, const QString &control, const QVariantMap &state);

private:
    void detach();
    void setAvailable(bool on);
    QDBusMessage call(const QString &method) const;

    QDBusConnection m_conn;
    QString m_service;
    bool m_attached = false;
    bool m_hasOverride = false;
    QString m_override;
    bool m_available = false;
    qint64 m_pid = 0;
    QVariantMap m_context;
    qint64 m_contextSerial = -1;
    uint m_seq = 0;
    quint64 m_generation = 0;
    QDBusServiceWatcher *m_watcher = nullptr;
};

} // namespace cs
