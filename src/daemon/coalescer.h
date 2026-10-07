// SPDX-License-Identifier: GPL-2.0-or-later
// Merges knob detents per control so the consumer never sees more than one
// outstanding update per control, and never more often than minInterval.
#pragma once

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

    // payload is opaque and forwarded with the flush (latest wins).
    void add(const QString &key, double delta, const QVariantMap &payload = {});
    void ack(const QString &key);
    void clear();
    double pending(const QString &key) const;
    int flushCount() const { return m_flushes; }

Q_SIGNALS:
    void flushed(const QString &key, double delta, int merged, const QVariantMap &payload);

private:
    struct Slot {
        double pending = 0;
        int merged = 0;
        QVariantMap payload;
        QElapsedTimer lastFlush;
        QElapsedTimer inFlightSince;
        bool inFlight = false;
        QTimer *timer = nullptr;
    };
    void tryFlush(const QString &key);
    void schedule(Slot &s, const QString &key, int ms);

    QHash<QString, Slot> m_slots;
    int m_minInterval = 8;
    int m_ackTimeout = 60;
    bool m_requireAck = true;
    int m_flushes = 0;
};

} // namespace cs
