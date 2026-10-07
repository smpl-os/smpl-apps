// SPDX-License-Identifier: GPL-2.0-or-later
#include "coalescer.h"

#include <QTimer>
#include <cmath>

namespace cs {

DeltaCoalescer::DeltaCoalescer(QObject *parent)
    : QObject(parent)
{
}

bool DeltaCoalescer::add(const QString &key, double delta, const QVariantMap &payload)
{
    if (!m_slots.contains(key) && m_slots.size() >= m_maxKeys) {
        return false;
    }
    Slot &s = m_slots[key];
    s.pending += delta;
    s.merged += 1;
    s.payload = payload;
    tryFlush(key);
    return true;
}

void DeltaCoalescer::end(const QString &key, const QVariantMap &endPayload, bool dropPending)
{
    auto it = m_slots.find(key);
    if (it == m_slots.end()) {
        return;
    }
    it->endRequested = true;
    it->endPayload = endPayload;
    if (dropPending) {
        it->pending = 0;
        it->merged = 0;
    }
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

void DeltaCoalescer::removeSlot(const QString &key)
{
    auto it = m_slots.find(key);
    if (it == m_slots.end()) {
        return;
    }
    if (it->timer) {
        it->timer->stop();
        it->timer->deleteLater();  // may be running its own timeout
    }
    m_slots.erase(it);
}

void DeltaCoalescer::clear()
{
    const auto keys = m_slots.keys();
    for (const auto &k : keys) {
        removeSlot(k);
    }
}

void DeltaCoalescer::clearExcept(const QSet<QString> &keep)
{
    const auto keys = m_slots.keys();
    for (const auto &k : keys) {
        if (!keep.contains(k)) {
            removeSlot(k);
        }
    }
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
    if (s.merged == 0 && !s.endRequested) {
        return;
    }
    // An end barrier is sent at once: the transport keeps per-sender order, so it
    // still arrives after the in-flight update, and it closes the gesture before
    // any discrete operation the caller sends next.
    if (s.inFlight && !s.endRequested) {
        const int waited = int(s.inFlightSince.elapsed());
        if (waited < m_ackTimeout) {
            schedule(s, key, m_ackTimeout - waited);
            return;
        }
        s.inFlight = false;  // ack lost or consumer slow: do not stall the knob forever
    }
    if (s.lastFlush.isValid() && !s.endRequested) {
        const int since = int(s.lastFlush.elapsed());
        if (since < m_minInterval) {
            schedule(s, key, m_minInterval - since);
            return;
        }
    }
    const double delta = s.pending;
    const int merged = s.merged;
    s.pending = 0;
    s.merged = 0;
    if (s.endRequested) {
        // The barrier goes out even with a zero net delta, then the key is done.
        const QVariantMap payload = s.endPayload;
        removeSlot(key);
        ++m_flushes;
        Q_EMIT flushed(key, delta, merged, payload, true);
        return;
    }
    const QVariantMap payload = s.payload;
    if (std::abs(delta) < 1e-12) {
        return;  // detents cancelled out
    }
    s.lastFlush.start();
    if (m_requireAck) {
        s.inFlight = true;
        s.inFlightSince.start();
    }
    ++m_flushes;
    Q_EMIT flushed(key, delta, merged, payload, false);
}

} // namespace cs
