// SPDX-License-Identifier: GPL-2.0-or-later
#include "engine.h"
#include "kdenlivecontract.h"

#include <QCoreApplication>
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

QString defaultTargetPath(const QString &name)
{
    if (name == contract::kParamNudge || name == contract::kCmdParamReset) {
        return QStringLiteral("param.target");
    }
    if (name == contract::kColorWheel || name == contract::kCmdWheelReset) {
        return QStringLiteral("colorWheel.target");
    }
    if (name == contract::kAudioGain) {
        return QStringLiteral("audio.target");
    }
    if (name == contract::kTrim) {
        return QStringLiteral("edit.target");
    }
    if (name == contract::kCmdTrackSet) {
        return QStringLiteral("timeline.track.target");
    }
    return {};
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
    m_gestureTimer = new QTimer(this);
    m_gestureTimer->setInterval(50);
    connect(m_gestureTimer, &QTimer::timeout, this, &Engine::checkIdleGestures);
    connect(&m_coalescer, &DeltaCoalescer::flushed, this, &Engine::onFlush);
    if (m_kd) {
        // Acks are correlated by the client to the exact key (lease/sequence/target);
        // only that key is released.
        connect(m_kd, &KdenliveClient::controlAcked, this, [this](const QString &key, const QVariantMap &) { m_coalescer.ack(key); });
        connect(m_kd, &KdenliveClient::refused, this, [this](const QString &what, const QString &code, const QString &msg) {
            sayOnce(what + QLatin1Char('|') + code, QStringLiteral("Kdenlive refused %1: %2%3").arg(what, code, msg.isEmpty() ? QString() : QStringLiteral(" (%1)").arg(msg)));
            if (code == contract::err::StaleContext || code == contract::err::TargetNotFound) {
                dropPendingWork();  // the host already dropped this work; resync on the next input
            }
        });
        connect(m_kd, &KdenliveClient::actionFailed, this, [this](const QString &id) {
            // Interface proven absent at call time (stock Kdenlive): stock keys,
            // but only into the same Kdenlive window that was asked.
            const Fallback fb = m_actionFallback.take(id);
            if (fb.keys.isEmpty() || !kdenliveProfile() || fb.pid != m_window.pid || fb.address != m_window.address) {
                return;
            }
            say(QStringLiteral("action %1: interface absent, using stock shortcut").arg(id));
            enqueueTaps(QStringLiteral("action:") + id, 0, fb.keys, 1);
        });
        connect(m_kd, &KdenliveClient::stateChanged, this, [this](KdenliveClient::State) {
            dropPendingWork();
            m_said.clear();
        });
        connect(m_kd, &KdenliveClient::epochChanged, this, [this](quint64) {
            // Targets changed and the host has already invalidated pending work
            // and ended open gestures: never deliver queued deltas to a new target.
            dropPendingWork();
        });
    }
    setConfig(Config{});
}

void Engine::say(const QString &text)
{
    Q_EMIT message(text);
}

void Engine::sayOnce(const QString &key, const QString &text)
{
    if (!m_said.contains(key)) {
        m_said.insert(key);
        Q_EMIT message(text);
    }
}

void Engine::dropPendingWork()
{
    m_coalescer.clear();
    m_gestures.clear();
    m_gestureTimer->stop();
    m_tapQueue.clear();
}

void Engine::setConfig(const Config &cfg)
{
    m_cfg = cfg;
    dropPendingWork();
    m_coalescer.setMinIntervalMs(cfg.settings.coalesceMs);
    m_coalescer.setAckTimeoutMs(cfg.settings.ackTimeoutMs);
    m_coalescer.setMaxKeys(contract::kMaxPendingKeys);
    m_tapTimer->setInterval(qMax(1, 1000 / qMax(1, cfg.settings.keyRateHz)));
    m_profile = nullptr;
    setActiveWindow(m_window);
}

void Engine::setActiveWindow(const WindowInfo &w)
{
    const Profile *before = m_profile;
    const bool sameWindow = w.cls == m_window.cls && w.pid == m_window.pid && w.address == m_window.address;
    m_window = w;
    m_profile = m_cfg.profileFor(w.cls, w.title);
    if (m_profile != before || !sameWindow) {
        // Never deliver a knob's leftover motion to the next window, even when
        // both windows share a profile. Title-only changes keep it.
        dropPendingWork();
    }
    if (m_profile != before) {
        m_said.clear();
        Q_EMIT message(QStringLiteral("profile %1 for %2").arg(m_profile ? m_profile->name : QStringLiteral("(none)"), w.cls.isEmpty() ? QStringLiteral("(no window)") : w.cls));
    }
    if (m_kd) {
        if (!kdenliveProfile()) {
            m_kd->attachToPid(0);
        } else if (w.pid > 0) {
            m_kd->attachToPid(w.pid);
        }
        // pid 0 for a Kdenlive window is a provisional focus event; keep the
        // current attachment until the window query reports the real pid.
    }
}

bool Engine::kdenliveProfile() const
{
    return m_profile && m_profile->kdenlive;
}

bool Engine::kdenliveActive() const
{
    return kdenliveProfile() && m_kd && m_kd->isAvailable();
}

bool Engine::kdenliveStock() const
{
    return kdenliveProfile() && (!m_kd || m_kd->isAbsent());
}

QStringList Engine::turnSlots(const QString &control, int delta)
{
    return {control + QStringLiteral(".turn"), control + (delta > 0 ? QStringLiteral(".cw") : QStringLiteral(".ccw"))};
}

std::optional<Engine::Resolution> Engine::resolve(const QStringList &candidates) const
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
            if (!conditionMatches(l.when, ctx)) {
                continue;
            }
            for (const QString &slot : candidates) {
                if (l.bindings.contains(slot)) {
                    return Resolution{l.bindings.value(slot), p->name, l.name, slot};
                }
            }
        }
        for (const QString &slot : candidates) {
            if (p->bindings.contains(slot)) {
                return Resolution{p->bindings.value(slot), p->name, QString(), slot};
            }
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
    const QVariantMap ctx = kdenliveActive() ? m_kd->context() : QVariantMap{};
    for (auto it = opts.begin(); it != opts.end(); ++it) {
        const QString s = it.value().toString();
        if (it.value().typeId() == QMetaType::QString && s.startsWith(QLatin1String("$ctx:"))) {
            out.insert(it.key(), valueAtPath(ctx, s.mid(5)));
        } else if (it.value().typeId() == QMetaType::QString && s.startsWith(QLatin1Char('$'))) {
            out.insert(it.key(), modeValue(s.mid(1)));
        } else {
            out.insert(it.key(), it.value());
        }
    }
    return out;
}

QString Engine::resolveTarget(const Binding &b, const QString &name) const
{
    const QString path = b.targetFrom.isEmpty() ? defaultTargetPath(name) : b.targetFrom;
    if (path.isEmpty() || !m_kd) {
        return {};
    }
    return valueAtPath(m_kd->context(), path).toString();
}

void Engine::handle(const PadEvent &e)
{
    if (kdenliveProfile() && m_kd && !m_kd->isAvailable() && !m_kd->isAbsent()) {
        m_kd->retry();  // not answered definitively yet: ask again (rate-limited by the client)
    }
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
        const QStringList candidates = turnSlots(e.control, e.delta);
        const auto r = resolve(candidates);
        if (!r || !r->binding.isValid()) {
            return;  // unbound, or explicitly "none" at the winning level
        }
        if (r->slot == candidates.first()) {
            execute(*r, r->slot, e.delta, true, accel);
        } else {
            execute(*r, r->slot, std::abs(e.delta), true, accel);
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
    const bool kd = kdenliveProfile();
    switch (b.kind) {
    case Binding::None:
        return;
    case Binding::Keys:
        enqueueTaps(group, dir, b.keys, count);
        return;
    case Binding::Action:
        if (!kd || kdenliveStock()) {
            // Not Kdenlive, or plain stock Kdenlive: the action's stock shortcut.
            if (!b.keys.isEmpty()) {
                enqueueTaps(group, dir, b.keys, isTurn ? count : 1);
            } else if (kd) {
                sayOnce(QStringLiteral("nofallback|") + b.name, QStringLiteral("%1: no stock shortcut configured for %2").arg(slot, b.name));
            }
        } else if (kdenliveActive()) {
            if (!m_kd->supportsAction(b.name)) {
                sayOnce(QStringLiteral("action|") + b.name, QStringLiteral("Kdenlive does not offer action %1 to control surfaces").arg(b.name));
                return;
            }
            endAllGestures(false);  // discrete operations close editing gestures first
            m_actionFallback.insert(b.name, Fallback{b.keys, m_window.pid, m_window.address});
            for (int i = 0; i < (isTurn ? count : 1); ++i) {
                m_kd->triggerAction(b.name);
            }
        } else {
            sayOnce(QStringLiteral("pending"), QStringLiteral("Kdenlive has not answered yet; %1 dropped (no keyboard fallback)").arg(b.name));
        }
        return;
    case Binding::Control:
        executeControl(b, slot, group, dir, (isTurn ? detents : 1) * accel * b.scale);
        return;
    case Binding::Command:
        Q_EMIT runCommand(b.argv);
        return;
    case Binding::Cycle: {
        QString owner;
        const auto *modes = modesFor(b.name, &owner);
        if (!modes) {
            say(QStringLiteral("cycle: unknown mode %1").arg(b.name));
            return;
        }
        endAllGestures(false);  // options change: the next turn is a new gesture
        const QString key = owner + QLatin1Char('/') + b.name;
        m_modeIndex[key] = (m_modeIndex.value(key) + 1) % modes->value(b.name).size();
        const QString text = QStringLiteral("%1: %2").arg(b.label.isEmpty() ? b.name : b.label, modeValue(b.name));
        say(text);
        if (kdenliveActive()) {
            m_kd->notify(text);
        }
        return;
    }
    case Binding::Request:
        if (kdenliveActive()) {
            if (!m_kd->supportsCommand(b.name)) {
                sayOnce(QStringLiteral("command|") + b.name, QStringLiteral("Kdenlive does not offer command %1").arg(b.name));
                return;
            }
            QVariantMap args = expandOptions(b.options);
            const QString path = b.targetFrom.isEmpty() ? defaultTargetPath(b.name) : b.targetFrom;
            if (!path.isEmpty() && !args.contains(contract::kOptTarget)) {
                const QString target = valueAtPath(m_kd->context(), path).toString();
                if (target.isEmpty()) {
                    sayOnce(QStringLiteral("notarget|") + b.name, QStringLiteral("%1: no %2 in Kdenlive's context").arg(b.name, path));
                    return;
                }
                args.insert(contract::kOptTarget, target);
            }
            endAllGestures(false);
            m_kd->invoke(b.name, args);
        } else if (kdenliveStock() && !b.keys.isEmpty()) {
            enqueueTaps(group, dir, b.keys, 1);
        }
        return;
    }
}

void Engine::executeControl(const Binding &b, const QString &slot, const QString &group, int dir, double delta)
{
    if (!kdenliveProfile() || kdenliveStock()) {
        // Stock behaviour: [negative key, positive key] or one key.
        if (b.keys.size() >= 2) {
            enqueueTaps(group, delta < 0 ? -1 : 1, {b.keys.at(delta < 0 ? 0 : 1)}, qMax(1, int(std::lround(std::abs(delta)))));
        } else if (b.keys.size() == 1) {
            enqueueTaps(group, dir, b.keys, qMax(1, int(std::lround(std::abs(delta)))));
        }
        return;
    }
    if (!kdenliveActive()) {
        sayOnce(QStringLiteral("pending"), QStringLiteral("Kdenlive has not answered yet; %1 dropped (no keyboard fallback)").arg(b.name));
        return;
    }
    if (!m_kd->supportsControl(b.name)) {
        sayOnce(QStringLiteral("control|") + b.name, QStringLiteral("Kdenlive does not offer %1 yet").arg(b.name));
        return;
    }
    const QVariantMap opts = expandOptions(b.options);
    QString key;
    QVariantMap payloadOptions = opts;
    if (contract::isEditingControl(b.name)) {
        const QString target = resolveTarget(b, b.name);
        if (target.isEmpty()) {
            sayOnce(QStringLiteral("notarget|") + b.name, QStringLiteral("%1: no editing target in Kdenlive's context").arg(b.name));
            return;
        }
        const QString bindingId = slot + QLatin1Char('|') + b.name + QLatin1Char('|') + optionsKey(opts);
        const quint64 epoch = m_kd->epoch();
        auto it = m_gestures.find(bindingId);
        if (it != m_gestures.end() && (it->target != target || it->epoch != epoch || it->last.elapsed() > m_cfg.settings.gestureIdleMs)) {
            endGesture(bindingId, false);
            it = m_gestures.end();
        }
        if (it == m_gestures.end()) {
            Gesture g;
            g.id = QStringLiteral("cs-%1-%2").arg(QCoreApplication::applicationPid()).arg(++m_gestureCounter);
            g.key = bindingId + QStringLiteral("|") + g.id + QStringLiteral("|") + target;
            g.control = b.name;
            g.target = target;
            g.epoch = epoch;
            g.options = opts;
            it = m_gestures.insert(bindingId, g);
            m_gestureTimer->start();
        }
        it->last.start();
        key = it->key;
        payloadOptions.insert(contract::kOptTarget, target);
        payloadOptions.insert(contract::kOptGesture, it->id);
        payloadOptions.insert(contract::kOptPhase, QStringLiteral("update"));
    } else {
        key = b.name + QLatin1Char('|') + optionsKey(opts);
    }
    if (!m_coalescer.add(key, delta, QVariantMap{{QStringLiteral("name"), b.name}, {QStringLiteral("options"), payloadOptions}})) {
        sayOnce(QStringLiteral("limit"), QStringLiteral("too many pending controls; dropping input"));
    }
}

void Engine::endGesture(const QString &bindingId, bool dropPending)
{
    const auto it = m_gestures.constFind(bindingId);
    if (it == m_gestures.constEnd()) {
        return;
    }
    const Gesture g = *it;
    m_gestures.erase(it);
    QVariantMap opts = g.options;
    opts.insert(contract::kOptTarget, g.target);
    opts.insert(contract::kOptGesture, g.id);
    opts.insert(contract::kOptPhase, QStringLiteral("end"));
    m_coalescer.end(g.key, QVariantMap{{QStringLiteral("name"), g.control}, {QStringLiteral("options"), opts}}, dropPending);
    if (m_gestures.isEmpty()) {
        m_gestureTimer->stop();
    }
}

void Engine::endAllGestures(bool dropPending)
{
    const auto ids = m_gestures.keys();
    for (const auto &id : ids) {
        endGesture(id, dropPending);
    }
}

void Engine::checkIdleGestures()
{
    const auto ids = m_gestures.keys();
    for (const auto &id : ids) {
        if (m_gestures.value(id).last.elapsed() > m_cfg.settings.gestureIdleMs) {
            endGesture(id, false);
        }
    }
}

void Engine::onFlush(const QString &key, double delta, int merged, const QVariantMap &payload, bool isEnd)
{
    Q_UNUSED(merged)
    Q_UNUSED(isEnd)
    if (!kdenliveActive()) {
        return;
    }
    if (!m_kd->control(key, payload.value(QStringLiteral("name")).toString(), delta, payload.value(QStringLiteral("options")).toMap())) {
        m_coalescer.ack(key);  // not sent: do not wait for an ack that cannot come
    }
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
