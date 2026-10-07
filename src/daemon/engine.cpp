// SPDX-License-Identifier: GPL-2.0-or-later
#include "engine.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>
#include <algorithm>
#include <cmath>

namespace cs {

namespace {
constexpr int kMaxQueuedTaps = 48;

QString optionsKey(const QVariantMap &o)
{
    return o.isEmpty() ? QString() : QString::fromUtf8(QJsonDocument(QJsonObject::fromVariantMap(o)).toJson(QJsonDocument::Compact));
}
} // namespace

Engine::Engine(KeySink *keys, KdenliveClient *kdenlive, QObject *parent)
    : QObject(parent)
    , m_keys(keys)
    , m_kd(kdenlive)
{
    m_tapTimer = new QTimer(this);
    m_tapTimer->setTimerType(Qt::PreciseTimer);
    connect(m_tapTimer, &QTimer::timeout, this, &Engine::drainTap);
    connect(&m_coalescer, &DeltaCoalescer::flushed, this, &Engine::onFlush);
    if (m_kd) {
        connect(m_kd, &KdenliveClient::controlAcked, this, [this](const QString &name, const QVariantMap &) {
            const auto keys = m_inflight.take(name);
            for (const auto &k : keys) {
                m_coalescer.ack(k);
            }
        });
        connect(m_kd, &KdenliveClient::actionFailed, this, [this](const QString &id) {
            const auto fb = m_actionFallback.value(id);
            if (!fb.isEmpty()) {
                Q_EMIT message(QStringLiteral("action %1 unavailable, sending fallback keys").arg(id));
                enqueueTaps(QStringLiteral("action:") + id, 0, fb, 1);
            }
        });
        connect(m_kd, &KdenliveClient::availabilityChanged, this, [this](bool) { m_coalescer.clear(); m_inflight.clear(); });
    }
    setConfig(Config{});
}

void Engine::setConfig(const Config &cfg)
{
    m_cfg = cfg;
    m_coalescer.clear();
    m_inflight.clear();
    m_tapQueue.clear();
    m_coalescer.setMinIntervalMs(cfg.settings.coalesceMs);
    m_coalescer.setAckTimeoutMs(cfg.settings.ackTimeoutMs);
    m_tapTimer->setInterval(qMax(1, 1000 / qMax(1, cfg.settings.keyRateHz)));
    m_profile = nullptr;
    setActiveWindow(m_window);
}

void Engine::setActiveWindow(const WindowInfo &w)
{
    const Profile *before = m_profile;
    m_window = w;
    m_profile = m_cfg.profileFor(w.cls, w.title);
    if (m_profile != before) {
        // Never deliver a knob's leftover motion to the next application.
        m_coalescer.clear();
        m_inflight.clear();
        m_tapQueue.clear();
        Q_EMIT message(QStringLiteral("profile %1 for %2").arg(m_profile ? m_profile->name : QStringLiteral("(none)"), w.cls.isEmpty() ? QStringLiteral("(no window)") : w.cls));
    }
    if (m_kd) {
        m_kd->attachToPid(m_profile && m_profile->kdenlive ? w.pid : 0);
    }
}

bool Engine::kdenliveActive() const
{
    return m_kd && m_profile && m_profile->kdenlive && m_kd->isAvailable();
}

std::optional<Engine::Resolution> Engine::resolve(const QString &slot) const
{
    const QVariantMap ctx = kdenliveActive() ? m_kd->context() : QVariantMap{};
    QList<const Profile *> chain;
    if (m_profile) {
        chain << m_profile;
    }
    const Profile *global = m_cfg.globalProfile();
    if (global && (!m_profile || (m_profile->fallthrough && global != m_profile))) {
        chain << global;
    }
    for (const Profile *p : chain) {
        for (const Layer &l : p->layers) {
            if (l.bindings.contains(slot) && conditionMatches(l.when, ctx)) {
                return Resolution{l.bindings.value(slot), p->name, l.name};
            }
        }
        if (p->bindings.contains(slot)) {
            return Resolution{p->bindings.value(slot), p->name, QString()};
        }
    }
    return std::nullopt;
}

const QHash<QString, QStringList> *Engine::modesFor(const QString &mode, QString *owner) const
{
    for (const Profile *p : {m_profile, m_cfg.globalProfile()}) {
        if (p && p->modes.contains(mode)) {
            *owner = p->name;
            return &p->modes;
        }
    }
    return nullptr;
}

QString Engine::modeValue(const QString &mode) const
{
    QString owner;
    const auto *modes = modesFor(mode, &owner);
    if (!modes) {
        return {};
    }
    const QStringList values = modes->value(mode);
    return values.value(m_modeIndex.value(owner + QLatin1Char('/') + mode) % values.size());
}

QVariantMap Engine::expandOptions(const QVariantMap &opts) const
{
    QVariantMap out;
    for (auto it = opts.begin(); it != opts.end(); ++it) {
        const QString s = it.value().toString();
        if (it.value().typeId() == QMetaType::QString && s.startsWith(QLatin1Char('$'))) {
            out.insert(it.key(), modeValue(s.mid(1)));
        } else {
            out.insert(it.key(), it.value());
        }
    }
    return out;
}

void Engine::handle(const PadEvent &e)
{
    switch (e.type) {
    case PadEvent::KeyUp:
    case PadEvent::PressUp:
        return;  // bindings fire on press; the pad reports taps anyway
    case PadEvent::KeyDown:
        if (auto r = resolve(e.control)) {
            execute(*r, e.control, 1, false);
        }
        return;
    case PadEvent::PressDown: {
        const QString slot = e.control + QStringLiteral(".press");
        if (auto r = resolve(slot)) {
            execute(*r, slot, 1, false);
        }
        return;
    }
    case PadEvent::Turn: {
        double accel = 1.0;
        auto &t = m_lastTurn[e.control];
        if (t.isValid() && t.elapsed() < m_cfg.settings.accelWindowMs) {
            accel = m_cfg.settings.accelFactor;
        }
        t.start();
        const QString turnSlot = e.control + QStringLiteral(".turn");
        if (auto r = resolve(turnSlot); r && r->binding.isValid()) {
            execute(*r, turnSlot, e.delta, true, accel);
            return;
        }
        const QString dirSlot = e.control + (e.delta > 0 ? QStringLiteral(".cw") : QStringLiteral(".ccw"));
        if (auto r = resolve(dirSlot)) {
            execute(*r, dirSlot, std::abs(e.delta), true, accel);
        }
        return;
    }
    }
}

void Engine::execute(const Resolution &r, const QString &slot, double detents, bool isTurn, double accel)
{
    // Acceleration only scales continuous controls; discrete bindings stay one per detent.
    const Binding &b = r.binding;
    const QString group = slot.section(QLatin1Char('.'), 0, 0);
    const int dir = !isTurn ? 0 : (slot.endsWith(QLatin1String(".ccw")) ? -1 : slot.endsWith(QLatin1String(".cw")) ? 1 : (detents < 0 ? -1 : 1));
    const int count = qMax(1, int(std::lround(std::abs(detents))));
    switch (b.kind) {
    case Binding::None:
        return;
    case Binding::Keys:
        enqueueTaps(group, dir, b.keys, count);
        return;
    case Binding::Action:
        if (kdenliveActive()) {
            if (!b.keys.isEmpty()) {
                m_actionFallback.insert(b.name, b.keys);
            }
            for (int i = 0; i < (isTurn ? count : 1); ++i) {
                m_kd->triggerAction(b.name);
            }
        } else if (!b.keys.isEmpty()) {
            enqueueTaps(group, dir, b.keys, isTurn ? count : 1);
        } else {
            Q_EMIT message(QStringLiteral("%1: Kdenlive control interface not available, action %2 dropped").arg(slot, b.name));
        }
        return;
    case Binding::Control: {
        const double delta = detents * accel * b.scale;
        if (kdenliveActive()) {
            const QVariantMap opts = expandOptions(b.options);
            const QString key = b.name + QLatin1Char('|') + optionsKey(opts);
            m_coalescer.add(key, delta, QVariantMap{{QStringLiteral("name"), b.name}, {QStringLiteral("options"), opts}});
        } else if (b.keys.size() >= 2) {
            enqueueTaps(group, delta < 0 ? -1 : 1, {b.keys.at(delta < 0 ? 0 : 1)}, qMax(1, int(std::lround(std::abs(delta)))));
        } else if (b.keys.size() == 1) {
            enqueueTaps(group, dir, b.keys, count);
        }
        return;
    }
    case Binding::Command:
        Q_EMIT runCommand(b.argv);
        return;
    case Binding::Cycle: {
        QString owner;
        const auto *modes = modesFor(b.name, &owner);
        if (!modes) {
            Q_EMIT message(QStringLiteral("cycle: unknown mode %1").arg(b.name));
            return;
        }
        const QString key = owner + QLatin1Char('/') + b.name;
        m_modeIndex[key] = (m_modeIndex.value(key) + 1) % modes->value(b.name).size();
        const QString text = QStringLiteral("%1: %2").arg(b.label.isEmpty() ? b.name : b.label, modeValue(b.name));
        Q_EMIT message(text);
        if (kdenliveActive()) {
            m_kd->notify(text);
        }
        return;
    }
    case Binding::Request:
        if (kdenliveActive()) {
            m_kd->invoke(b.name, expandOptions(b.options));
        }
        return;
    }
}

void Engine::onFlush(const QString &key, double delta, int merged, const QVariantMap &payload)
{
    Q_UNUSED(merged)
    if (!kdenliveActive()) {
        return;
    }
    const QString name = payload.value(QStringLiteral("name")).toString();
    m_inflight[name].insert(key);
    m_kd->control(name, delta, payload.value(QStringLiteral("options")).toMap());
}

void Engine::enqueueTaps(const QString &group, int dir, const QList<KeyChord> &chords, int count)
{
    if (dir != 0) {
        // Reversing a knob cancels the not-yet-sent motion in the old direction.
        m_tapQueue.erase(std::remove_if(m_tapQueue.begin(), m_tapQueue.end(),
                                        [&](const Tap &t) { return t.group == group && t.dir == -dir; }),
                         m_tapQueue.end());
    }
    for (int i = 0; i < count; ++i) {
        for (const auto &c : chords) {
            if (m_tapQueue.size() >= kMaxQueuedTaps) {
                break;
            }
            m_tapQueue.append(Tap{group, dir, c});
        }
    }
    if (!m_tapTimer->isActive()) {
        // First tap goes out at once; the timer then acts as a cooldown so later
        // taps are spaced by 1/keyRateHz.
        drainTap();
        m_tapTimer->start();
    }
}

void Engine::drainTap()
{
    if (m_tapQueue.isEmpty()) {
        m_tapTimer->stop();
        return;
    }
    const Tap t = m_tapQueue.takeFirst();
    m_keys->tap(t.chord);
}

} // namespace cs
