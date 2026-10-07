// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QObject>
#include <QStringList>
#include <QVariantMap>

namespace cs {

// Client side of docs/kdenlive-api-contract.md. All calls are asynchronous;
// the daemon never blocks on Kdenlive.
class KdenliveClient : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;
    virtual bool isAvailable() const = 0;  // an instance implementing the contract is attached
    virtual QVariantMap context() const = 0;
    virtual void attachToPid(qint64 pid) = 0;  // 0 detaches
    virtual qint64 attachedPid() const = 0;
    virtual void triggerAction(const QString &id) = 0;
    virtual void control(const QString &name, double delta, const QVariantMap &options) = 0;
    virtual void invoke(const QString &command, const QVariantMap &args) = 0;
    virtual void notify(const QString &text) = 0;

Q_SIGNALS:
    void availabilityChanged(bool available);
    void contextChanged(const QVariantMap &context);
    void controlAcked(const QString &name, const QVariantMap &state);
    void actionFailed(const QString &id);
    void message(const QString &text);
};

// Records calls; used by tests, --dry-run and --simulate.
class FakeKdenliveClient : public KdenliveClient
{
    Q_OBJECT
public:
    explicit FakeKdenliveClient(bool print = false, QObject *parent = nullptr);
    bool isAvailable() const override { return m_available; }
    QVariantMap context() const override { return m_context; }
    void attachToPid(qint64 pid) override;
    qint64 attachedPid() const override { return m_pid; }
    void triggerAction(const QString &id) override;
    void control(const QString &name, double delta, const QVariantMap &options) override;
    void invoke(const QString &command, const QVariantMap &args) override;
    void notify(const QString &text) override;

    void setAvailable(bool on);
    void setContext(const QVariantMap &ctx);
    void setAutoAck(bool on) { m_autoAck = on; }
    void ackAll();

    QStringList calls;
    QList<double> controlDeltas;

private:
    void record(const QString &line);
    bool m_print;
    bool m_available = true;
    bool m_autoAck = true;
    qint64 m_pid = 0;
    QVariantMap m_context;
    QStringList m_unacked;
};

} // namespace cs
