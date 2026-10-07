// SPDX-License-Identifier: GPL-2.0-or-later
// Mock of the Kdenlive side of docs/kdenlive-api-contract.md. It keeps a small
// editing state so continuous controls have visible, testable effects.
#pragma once

#include <QDBusConnection>
#include <QDBusContext>
#include <QHash>
#include <QObject>
#include <QSet>
#include <QStringList>
#include <QVariantMap>

class QTimer;

namespace cs {

class MockKdenlive : public QObject, protected QDBusContext
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.kdenlive.ControlSurface1")
public:
    explicit MockKdenlive(QObject *parent = nullptr);
    bool registerOn(QDBusConnection connection);

    // Test/console helpers (not exported).
    void setContext(const QVariantMap &context);
    void setContextValue(const QString &key, const QVariant &value);
    QVariantMap context() const { return m_context; }
    QVariantMap state() const;
    void setApplyDelayMs(int ms) { m_applyDelayMs = ms; }
    void setPrint(bool on) { m_print = on; }
    int subscriberCount() const { return int(m_subscribers.size()); }
    int controlMessages() const { return m_controlMessages; }
    int applyBatches() const { return m_applyBatches; }
    int contextSignals() const { return m_contextSignals; }
    QStringList log;

public Q_SLOTS:
    QVariantMap Capabilities();
    QVariantMap Subscribe();
    void Unsubscribe();
    QVariantMap GetContext();
    QList<QVariantMap> ListActions();
    bool TriggerAction(const QString &id);
    Q_NOREPLY void Control(const QString &control, double delta, const QVariantMap &options, uint seq);
    bool SetControlValue(const QString &control, double value, const QVariantMap &options);
    QVariantMap Invoke(const QString &command, const QVariantMap &args);
    Q_NOREPLY void Notify(const QString &text, int timeoutMs);

Q_SIGNALS:
    void ContextChanged(const QVariantMap &context);
    void ControlAck(uint seq, const QString &control, const QVariantMap &state);
    void ActionsChanged();

private:
    struct Pending {
        QString control;
        QVariantMap options;
        double delta = 0;
        uint seq = 0;
    };
    void applyPending();
    QVariantMap applyControl(const QString &control, double delta, const QVariantMap &options);
    void emitContextSoon();
    void record(const QString &line);
    QString valueAtFocus() const;

    QVariantMap m_context;
    qint64 m_serial = 0;
    QSet<QString> m_subscribers;
    QHash<QString, Pending> m_pending;
    QStringList m_pendingOrder;
    bool m_applyScheduled = false;
    int m_applyDelayMs = 0;
    QTimer *m_contextTimer;
    bool m_contextDirty = false;
    bool m_print = false;
    int m_controlMessages = 0;
    int m_applyBatches = 0;
    int m_contextSignals = 0;

    // editing state
    int m_position = 0;
    int m_duration = 25 * 600;
    int m_shuttle = 0;
    double m_zoom = 10;
    double m_scroll = 0;
    int m_track = 0;
    int m_trim = 0;
    double m_gainDb = 0;
    QVariantMap m_wheels;  // lift/gamma/gain -> {r,g,b}
    QVariantMap m_params;  // name -> value
    QStringList m_paramOrder;
    QVariantMap m_curve;   // value/time offsets of selected automation points
    QStringList m_triggered;
};

} // namespace cs
