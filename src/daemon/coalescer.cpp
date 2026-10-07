// SPDX-License-Identifier: GPL-2.0-or-later
#include "coalescer.h"

#include <QTimer>
#include <cmath>

namespace cs {

DeltaCoalescer::DeltaCoalescer(QObject *parent)
    : QObject(parent)
{
}

void DeltaCoalescer::add(const QString &key, double delta, const QVariantMap &payload)
{
    Slot &s = m_slots[key];
    s.pending += delta;
    s.merged += 1;
    s.payload = payload;
    tryFlush(key);
}

void DeltaCoalescer::ack(const QString &key)
{
    auto it = m_slots.find(key);
    if (it == m_slots.end()) {
        return;
    }
    it->inFlight = false;
    tryFlush(key);
}

void DeltaCoalescer::clear()
{
    for (auto &s : m_slots) {
        delete s.timer;
    }
    m_slots.clear();
}

double DeltaCoalescer::pending(const QString &key) const
{
    return m_slots.value(key).pending;
}

void DeltaCoalescer::schedule(Slot &s, const QString &key, int ms)
{
    if (!s.timer) {
        s.timer = new QTimer(this);
        s.timer->setSingleShot(true);
        s.timer->setTimerType(Qt::PreciseTimer);
        connect(s.timer, &QTimer::timeout, this, [this, key] { tryFlush(key); });
    }
    if (!s.timer->isActive() || s.timer->remainingTime() > ms) {
        s.timer->start(qMax(0, ms));
    }
}

void DeltaCoalescer::tryFlush(const QString &key)
{
    auto it = m_slots.find(key);
    if (it == m_slots.end()) {
        return;
    }
    Slot &s = *it;
    if (s.merged == 0) {
        return;
    }
    if (s.inFlight) {
        const int waited = int(s.inFlightSince.elapsed());
        if (waited < m_ackTimeout) {
            schedule(s, key, m_ackTimeout - waited);
            return;
        }
        s.inFlight = false;  // ack lost or consumer slow: do not stall the knob forever
    }
    if (s.lastFlush.isValid()) {
        const int since = int(s.lastFlush.elapsed());
        if (since < m_minInterval) {
            schedule(s, key, m_minInterval - since);
            return;
        }
    }
    const double delta = s.pending;
    const int merged = s.merged;
    const QVariantMap payload = s.payload;
    s.pending = 0;
    s.merged = 0;
    if (std::abs(delta) < 1e-12) {
        return;  // detents cancelled out
    }
    s.lastFlush.start();
    if (m_requireAck) {
        s.inFlight = true;
        s.inFlightSince.start();
    }
    ++m_flushes;
    Q_EMIT flushed(key, delta, merged, payload);
}

} // namespace cs
