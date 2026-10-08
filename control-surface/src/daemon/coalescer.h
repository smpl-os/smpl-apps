// SPDX-License-Identifier: GPL-2.0-or-later
// Merges knob detents per key so the consumer never sees more than one
// outstanding update per key, and never more often than minInterval. An
// explicit end is a barrier: it is sent immediately (ordered after any
// in-flight message by the transport) even when the net delta is zero, and
// retires the key.
#pragma once

#include <QSet>

#include <QElapsedTimer>
#include <QHash>
#include <QObject>
#include <QVariantMap>

class QTimer;

namespace cs {

class DeltaCoalescer : public QObject
{
    Q_OBJECT
public:
    explicit DeltaCoalescer(QObject *parent = nullptr);
    void setMinIntervalMs(int ms) { m_minInterval = ms; }
    void setAckTimeoutMs(int ms) { m_ackTimeout = ms; }
    void setRequireAck(bool on) { m_requireAck = on; }
    void setMaxKeys(int n) { m_maxKeys = n; }

    // payload is opaque and forwarded with the flush (latest wins). Returns
    // false when a new key would exceed maxKeys (nothing queued).
    bool add(const QString &key, double delta, const QVariantMap &payload = {});
    // Queue the end barrier for key with endPayload. dropPending discards
    // not-yet-sent deltas (target changed). Unknown keys are ignored.
    void end(const QString &key, const QVariantMap &endPayload, bool dropPending);
    void ack(const QString &key);
    // Forget key without sending anything (the consumer already ended it).
    void drop(const QString &key) { removeSlot(key); }
    void clear();
    void clearExcept(const QSet<QString> &keep);
    bool contains(const QString &key) const { return m_slots.contains(key); }
    double pending(const QString &key) const;
    int keyCount() const { return int(m_slots.size()); }
    int flushCount() const { return m_flushes; }

Q_SIGNALS:
    void flushed(const QString &key, double delta, int merged, const QVariantMap &payload, bool isEnd);

private:
    struct Slot {
        double pending = 0;
        int merged = 0;
        QVariantMap payload;
        bool endRequested = false;
        QVariantMap endPayload;
        QElapsedTimer lastFlush;
        QElapsedTimer inFlightSince;
        bool inFlight = false;
        QTimer *timer = nullptr;
    };
    void tryFlush(const QString &key);
    void schedule(Slot &s, const QString &key, int ms);
    void removeSlot(const QString &key);

    QHash<QString, Slot> m_slots;
    int m_minInterval = 8;
    int m_ackTimeout = 60;
    bool m_requireAck = true;
    int m_maxKeys = 64;
    int m_flushes = 0;
};

} // namespace cs
