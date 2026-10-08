// SPDX-License-Identifier: GPL-2.0-or-later
#include "engine.h"
#include "installed.h"
#include "kdenlivecontract.h"

#include <QCoreApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
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
        return QStringLiteral("timeline.clipGain.target");  // track gain: targetFrom timeline.track.gain.target
    }
    if (name == contract::kTrim) {
        return QStringLiteral("timeline.trim.target");
    }
    if (name == contract::kCmdTrackSet) {
        return QStringLiteral("timeline.track.target");
    }
    return {};
}
// The descriptor (a map with this "target") anywhere in the context.
QVariantMap descriptorFor(const QVariant &v, const QString &target, int depth = 0)
{
    if (depth > 4 || target.isEmpty()) {
        return {};
    }
    if (v.typeId() == QMetaType::QVariantMap) {
        const QVariantMap m = v.toMap();
        if (m.value(contract::kOptTarget).typeId() == QMetaType::QString && m.value(contract::kOptTarget).toString() == target) {
            return m;
        }
        for (auto it = m.cbegin(); it != m.cend(); ++it) {
            const QVariantMap d = descriptorFor(it.value(), target, depth + 1);
            if (!d.isEmpty()) {
                return d;
            }
        }
    } else if (v.typeId() == QMetaType::QVariantList) {
        for (const auto &e : v.toList()) {
            const QVariantMap d = descriptorFor(e, target, depth + 1);
            if (!d.isEmpty()) {
                return d;
            }
        }
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
    m_navTimer = new QTimer(this);
    m_navTimer->setSingleShot(true);
    m_navTimer->setInterval(250);
    connect(m_navTimer, &QTimer::timeout, this, &Engine::releaseNavigation);
    m_stepTimer = new QTimer(this);
    m_stepTimer->setSingleShot(true);
    connect(m_stepTimer, &QTimer::timeout, this, [this] {
        if (m_steps.isEmpty()) {
            return;
        }
        const PendingStep s = m_steps.takeFirst();
        execute(s.r, s.slot, s.detents, s.isTurn, s.accel);
        if (!m_steps.isEmpty()) {
            m_stepTimer->start(m_steps.first().delayMs);
        }
    });
    m_rollTimer = new QTimer(this);
    m_rollTimer->setSingleShot(true);
    m_rollTimer->setInterval(kRollMs);
    connect(m_rollTimer, &QTimer::timeout, this, [this] {
        const auto r = m_rollTap;
        m_rollControl.clear();
        m_rollTap.reset();
        if (r) {
            fireResolved(*r);
        }
    });
    m_gestureTimer = new QTimer(this);
    m_gestureTimer->setInterval(50);
    connect(m_gestureTimer, &QTimer::timeout, this, &Engine::checkIdleGestures);
    connect(&m_coalescer, &DeltaCoalescer::flushed, this, &Engine::onFlush);
    if (m_kd) {
        // Acks are correlated by the client to the exact key (lease/sequence/target);
        // only that key is released.
        connect(m_kd, &KdenliveClient::controlAcked, this, [this](const QString &key, const QVariantMap &outcome) {
            if (m_navInFlight.contains(key)) {
                const quint64 sentEpoch = m_navInFlight.take(key);
                const bool changed = outcome.value(QStringLiteral("ok")).toBool()
                    && outcome.value(QStringLiteral("result")).toMap().value(QStringLiteral("changed")).toBool();
                if (changed && m_kd->epoch() == sentEpoch) {
                    m_navAwaitingEpoch.insert(key);  // released by the new epoch (or the fallback timer)
                    m_navTimer->start();
                    return;
                }
            }
            m_coalescer.ack(key);
        });
        connect(m_kd, &KdenliveClient::refused, this, [this](const QString &what, const QString &code, const QString &msg) {
            sayOnce(what + QLatin1Char('|') + code, QStringLiteral("Kdenlive refused %1: %2%3").arg(what, code, msg.isEmpty() ? QString() : QStringLiteral(" (%1)").arg(msg)));
            const bool trimming = !m_kd->actionEnabled(QStringLiteral("monitor_play")) || m_kd->context().value(QStringLiteral("tool")).toString() == QLatin1String("slip");
            if (code == contract::err::Busy && trimming && startsPlayback(what)) {
                // MR1a: the project monitor's trimming preview refuses playback (Kdenlive
                // lists the playback actions disabled); it is not a writer conflict.
                sayOnce(QStringLiteral("trimming|") + what,
                        QStringLiteral("%1: Kdenlive's trimming preview (Slip tool) blocks playback; switch to the Selection tool (select_tool) first").arg(what));
            }
            if (code == contract::err::StaleContext || code == contract::err::TargetNotFound) {
                dropPendingWork();  // the host already dropped this work; resync on the next input
            }
        });
        connect(m_kd, &KdenliveClient::actionFailed, this, [this](const QString &id) {
            // Interface proven absent at call time (stock Kdenlive): stock keys,
            // but only into the same Kdenlive window that was asked.
            const Fallback fb = m_actionFallback.take(id);
            if (!kdenliveProfile() || fb.pid != m_window.pid || fb.address != m_window.address) {
                return;
            }
            if (!m_profile->keyFallback) {
                noticeAbsent(id);
                return;
            }
            if (fb.keys.isEmpty()) {
                return;
            }
            say(QStringLiteral("action %1: interface absent, using stock shortcut").arg(id));
            enqueueTaps(QStringLiteral("action:") + id, 0, fb.keys, 1);
        });
        connect(m_kd, &KdenliveClient::stateChanged, this, [this](KdenliveClient::State) {
            dropPendingWork();
            m_said.clear();
            applyAutoModes();  // its context is known now
        });
        connect(m_kd, &KdenliveClient::contextChanged, this, [this](const QVariantMap &ctx) {
            // A seek ends a multi-key edit in the host (its captured frame moved):
            // forget it here too, without sending, so the next turn uses a fresh id.
            const auto ids = m_gestures.keys();
            for (const auto &id : ids) {
                const Gesture g = m_gestures.value(id);
                if (g.frameBound && ctx.value(QStringLiteral("position")) != g.position) {
                    m_gestures.remove(id);
                    m_coalescer.drop(g.key);
                }
            }
            applyAutoModes();
        });
        connect(m_kd, &KdenliveClient::epochChanged, this, [this](quint64) {
            // Targets changed and the host has already invalidated pending work
            // and ended open gestures: never deliver queued deltas to a new target.
            // Relative navigation that caused this change keeps its queued steps.
            const QSet<QString> keep = m_navAwaitingEpoch;
            m_coalescer.clearExcept(keep);
            m_gestures.clear();
            m_gestureTimer->stop();
            m_tapQueue.clear();
            m_navInFlight.clear();
            releaseNavigation();
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

void Engine::releaseNavigation()
{
    m_navTimer->stop();
    const QSet<QString> keys = m_navAwaitingEpoch;
    m_navAwaitingEpoch.clear();
    for (const auto &k : keys) {
        m_coalescer.ack(k);  // flushes queued steps with the current epoch
    }
}

void Engine::dropPendingWork()
{
    m_steps.clear();  // a sequence's later steps belong to the window it started in
    if (m_stepTimer) {
        m_stepTimer->stop();
    }
    m_navInFlight.clear();
    m_navAwaitingEpoch.clear();
    if (m_navTimer) {
        m_navTimer->stop();
    }
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
    clearHeld();
    // Rule state survives a reload (a save in Settings applies the same rules
    // again): otherwise every matching rule would fire anew, losing what
    // "restore" goes back to and overriding manual choices. Rules that are gone
    // are forgotten.
    QSet<QString> rules;
    for (const Profile &p : m_cfg.profiles) {
        for (const Profile::ModeRule &r : p.autoModes) {
            rules.insert(p.name + QLatin1Char('#') + r.name);
        }
    }
    for (auto it = m_ruleState.begin(); it != m_ruleState.end();) {
        it = rules.contains(it.key()) ? std::next(it) : m_ruleState.erase(it);
    }
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
        clearHeld();  // a press deferred for another profile's shift binding must not fire here
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
    applyAutoModes();
    Q_EMIT resolutionChanged();
}

bool Engine::kdenliveProfile() const
{
    return m_profile && m_profile->kdenlive;
}

bool Engine::kdenliveAttachedToFocus() const
{
    // During a provisional focus event (pid unknown) or before re-attaching, the
    // client state belongs to another instance: treat it as unanswered.
    return m_kd && m_window.pid > 0 && m_kd->attachedPid() == m_window.pid;
}

bool Engine::kdenliveActive() const
{
    return kdenliveProfile() && kdenliveAttachedToFocus() && m_kd->isAvailable();
}

bool Engine::kdenliveAbsent() const
{
    return kdenliveProfile() && (!m_kd || (kdenliveAttachedToFocus() && m_kd->isAbsent()));
}

bool Engine::kdenliveStock() const
{
    // API-only by default: stock shortcuts are typed only when the profile opts in.
    return kdenliveAbsent() && m_profile->keyFallback;
}

QString Engine::absentNotice()
{
    return QStringLiteral(
        "Kdenlive control interface not enabled: nothing sent. Enable \"Control surface interface\" in Kdenlive "
        "(setting enableControlSurfaceInterface, off by default), or set \"keyFallback\": true in the profile to type stock shortcuts.");
}

void Engine::noticeAbsent(const QString &what)
{
    // Once per attachment (m_said is cleared when the client state or profile changes).
    const QString key = QStringLiteral("absent|%1").arg(m_kd ? m_kd->attachedPid() : 0);
    if (!m_said.contains(key)) {
        m_said.insert(key);
        Q_EMIT message(QStringLiteral("%1 (%2)").arg(absentNotice(), what));
        Q_EMIT notice(absentNotice());
    }
}

bool Engine::startsPlayback(const QString &what)
{
    static const QStringList ids{QStringLiteral("monitor_play"),       QStringLiteral("monitor_seek_backward"), QStringLiteral("monitor_seek_forward"),
                                 QStringLiteral("monitor_play_zone"),  QStringLiteral("monitor_play_zone_cursor"), QStringLiteral("monitor_loop_zone"),
                                 QStringLiteral("monitor_loop_clip")};
    return what == contract::kShuttle || ids.contains(what);
}

QStringList Engine::shiftSlots(const QString &control, int delta)
{
    return {control + QStringLiteral(".shift.turn"), control + (delta > 0 ? QStringLiteral(".shift.cw") : QStringLiteral(".shift.ccw"))};
}

void Engine::clearHeld()
{
    // Deferred taps belong to the profile they were deferred in; what is
    // physically down (m_down) stays down.
    m_deferred.clear();
    m_shiftTurned.clear();
    m_usedWhileHeld.clear();
    m_rollTimer->stop();
    m_rollControl.clear();
    m_rollTap.reset();
}

void Engine::fireDeferred(const QString &slot)
{
    if (auto r = resolve(slot)) {
        fireResolved(*r);
    }
}

void Engine::fireResolved(const Resolution &r)
{
    m_inRelease = true;
    execute(r, r.slot, 1, false);
    m_inRelease = false;
}

void Engine::releaseAll()
{
    clearHeld();
    const bool had = !m_down.isEmpty();
    m_down.clear();
    if (!m_cheatsheetHold.isEmpty()) {
        m_cheatsheetHold.clear();
        Q_EMIT cheatsheetRequested(QStringLiteral("hide"));
    }
    if (had) {
        Q_EMIT resolutionChanged();
    }
}

bool Engine::isHeldModifier(const QString &control) const
{
    const Profile *global = m_cfg.globalProfile();
    return (m_profile && m_profile->heldControls.contains(control))
        || (global && (!m_profile || m_profile->fallthrough) && global->heldControls.contains(control));
}

QStringList Engine::heldModifiers() const
{
    QStringList out;
    for (const QString &c : m_down) {
        if (isHeldModifier(c)) {
            out << c;
        }
    }
    std::sort(out.begin(), out.end(), [](const QString &a, const QString &b) {
        return a.length() != b.length() ? a.length() < b.length() : a < b;  // key2 before key10
    });
    return out;
}

void Engine::setHeldForPreview(const QStringList &controls)
{
    m_down = QSet<QString>(controls.cbegin(), controls.cend());
    Q_EMIT resolutionChanged();
}

void Engine::noteUse(const QString &control)
{
    // Another input while a held-layer key is down: that key was used as a
    // modifier, so its own tap does not fire on release.
    for (const QString &c : std::as_const(m_down)) {
        if (c != control) {
            m_usedWhileHeld.insert(c);
        }
    }
}

QStringList Engine::turnSlots(const QString &control, int delta)
{
    return {control + QStringLiteral(".turn"), control + (delta > 0 ? QStringLiteral(".cw") : QStringLiteral(".ccw"))};
}

std::optional<Engine::Resolution> Engine::resolve(const QStringList &candidates) const
{
    QVariantMap ctx = kdenliveActive() ? m_kd->context() : QVariantMap{};
    ctx.insert(QStringLiteral("$mode"), modeContext());  // layers may depend on daemon modes
    QList<const Profile *> chain;
    if (m_profile) {
        chain << m_profile;
    }
    const Profile *global = m_cfg.globalProfile();
    if (global && (!m_profile || (m_profile->fallthrough && global != m_profile))) {
        chain << global;
    }
    auto fromLayer = [&](const Profile *p, const Layer &l) -> std::optional<Resolution> {
        if (!l.heldMatches(m_down) || !conditionMatches(l.when, ctx)) {
            return std::nullopt;
        }
        for (const QString &slot : candidates) {
            // A binding whose "ifInstalled" app is missing is skipped: the slot falls through.
            const auto it = l.bindings.constFind(slot);
            if (it != l.bindings.cend() && bindingAvailable(*it)) {
                return Resolution{*it, p->name, l.name, slot};
            }
        }
        return std::nullopt;
    };
    // "held" is a condition like any other: the first matching layer in list
    // order wins (list a held layer first to let it override), then the
    // profile's bindings, then the global profile's.
    for (const Profile *p : chain) {
        for (const Layer &l : p->layers) {
            if (auto r = fromLayer(p, l)) {
                return r;
            }
        }
        for (const QString &slot : candidates) {
            const auto it = p->bindings.constFind(slot);
            if (it != p->bindings.cend() && bindingAvailable(*it)) {
                return Resolution{*it, p->name, QString(), slot};
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

bool Engine::setModeValue(const QString &mode, const QString &value)
{
    QString owner;
    const auto *modes = modesFor(mode, &owner);
    if (!modes) {
        return false;
    }
    const int index = modes->value(mode).indexOf(value);
    const QString key = owner + QLatin1Char('/') + mode;
    if (index < 0 || m_modeIndex.value(key) == index) {
        return false;
    }
    m_modeIndex[key] = index;
    return true;
}

void Engine::stepMode(const QString &mode, int step)
{
    QString owner;
    const auto *modes = modesFor(mode, &owner);
    if (!modes || modes->value(mode).isEmpty()) {
        return;
    }
    const int n = int(modes->value(mode).size());
    const QString key = owner + QLatin1Char('/') + mode;
    m_modeIndex[key] = ((m_modeIndex.value(key) + step) % n + n) % n;
}

void Engine::announceMode(const QString &mode, const QString &label, const std::optional<QString> &bindingNotify)
{
    // The binding's "notify", else the mode's, else "{label}: {value}".
    QString tmpl = QStringLiteral("{label}: {value}");
    QString owner;
    if (bindingNotify) {
        tmpl = *bindingNotify;
    } else if (modesFor(mode, &owner)) {
        for (const Profile *p : {m_profile, m_cfg.globalProfile()}) {
            if (p && p->name == owner && p->modeNotify.contains(mode)) {
                tmpl = p->modeNotify.value(mode);
            }
        }
    }
    const QString value = modeValue(mode);
    QString text = tmpl;
    static const QRegularExpression valueOr(QStringLiteral("\\{value\\|([^}]*)\\}"));
    for (auto m = valueOr.globalMatch(tmpl); m.hasNext();) {
        const auto match = m.next();
        text.replace(match.captured(0), value.isEmpty() ? match.captured(1) : value);
    }
    text.replace(QStringLiteral("{value}"), value).replace(QStringLiteral("{mode}"), mode).replace(QStringLiteral("{label}"), label.isEmpty() ? mode : label);
    if (text.isEmpty()) {
        say(QStringLiteral("%1 = %2").arg(mode, value));  // log only
        return;
    }
    say(text);
    if (kdenliveActive()) {
        m_kd->notify(text);
    }
}

void Engine::applyAutoModes()
{
    if (m_applyingRules || !m_profile || m_profile->autoModes.isEmpty()) {
        return;
    }
    if (m_profile->kdenlive && !kdenliveActive()) {
        return;  // no context yet: decide nothing (and restore nothing)
    }
    m_applyingRules = true;
    bool any = false;
    for (int pass = 0; pass < 4; ++pass) {  // a rule may enable another through a mode
        bool changed = false;
        QVariantMap ctx = kdenliveActive() ? m_kd->context() : QVariantMap{};
        ctx.insert(QStringLiteral("$mode"), modeContext());
        for (int i = 0; i < m_profile->autoModes.size(); ++i) {
            const Profile::ModeRule &rule = m_profile->autoModes.at(i);
            RuleState &st = m_ruleState[m_profile->name + QLatin1Char('#') + rule.name];
            const bool on = conditionMatches(rule.when, ctx);
            if (on == st.on) {
                continue;
            }
            st.on = on;
            if (on) {
                st.before.clear();
                for (const auto &[mode, value] : rule.set) {
                    st.before.insert(mode, modeValue(mode));
                    if (setModeValue(mode, value)) {
                        changed = true;
                        if (rule.notify) {
                            announceMode(mode, QString(), std::nullopt);
                        }
                    }
                }
            } else if (rule.restore) {
                // Back, unless the user changed it meanwhile (the manual choice wins).
                for (const auto &[mode, value] : rule.set) {
                    if (modeValue(mode) == value && st.before.contains(mode) && setModeValue(mode, st.before.value(mode))) {
                        changed = true;
                        if (rule.notify) {
                            announceMode(mode, QString(), std::nullopt);
                        }
                    }
                }
            }
            if (changed) {
                ctx.insert(QStringLiteral("$mode"), modeContext());
            }
        }
        if (!changed) {
            break;
        }
        any = true;
    }
    m_applyingRules = false;
    if (any) {
        endAllGestures(false);
        Q_EMIT resolutionChanged();
    }
}

QVariantMap Engine::modeContext() const
{
    QVariantMap out;
    for (const Profile *p : {m_cfg.globalProfile(), m_profile}) {  // the active profile wins
        if (!p) {
            continue;
        }
        for (auto it = p->modes.cbegin(); it != p->modes.cend(); ++it) {
            out.insert(it.key(), modeValue(it.key()));
        }
    }
    return out;
}

QVariantMap Engine::expandOptions(const QVariantMap &opts, QString *missing) const
{
    QVariantMap out;
    const QVariantMap ctx = kdenliveActive() ? m_kd->context() : QVariantMap{};
    for (auto it = opts.begin(); it != opts.end(); ++it) {
        const QString s = it.value().toString();
        const bool negate = s.startsWith(QLatin1String("$!ctx:"));
        if (it.value().typeId() == QMetaType::QString && (negate || s.startsWith(QLatin1String("$ctx:")))) {
            const QString path = s.mid(negate ? 6 : 5);
            const QVariant v = valueAtPath(ctx, path);
            if (!v.isValid() && missing && missing->isEmpty()) {
                *missing = path;  // e.g. a toggle whose current state is not published
            }
            out.insert(it.key(), negate && v.isValid() ? QVariant(!v.toBool()) : v);
        } else if (it.value().typeId() == QMetaType::QString && s.startsWith(QLatin1Char('$'))) {
            out.insert(it.key(), modeValue(s.mid(1)));
        } else {
            out.insert(it.key(), it.value());
        }
    }
    return out;
}

QString Engine::resolveTarget(const Binding &b, const QString &name, const QVariantMap &options) const
{
    const QString path = b.targetFrom.isEmpty() ? defaultTargetPath(name) : b.targetFrom;
    if (path.isEmpty() || !m_kd) {
        return {};
    }
    const QVariantMap ctx = m_kd->context();
    const bool wheelControl = name == contract::kColorWheel || name == contract::kCmdWheelReset;
    const QString wheel = options.value(QStringLiteral("wheel")).toString();
    auto wheelOf = [](const QVariantMap &descriptor) { return descriptor.value(QStringLiteral("wheel")).toString(); };
    if (wheelControl && b.targetFrom.isEmpty() && !wheel.isEmpty()) {
        // MR2: the focused Lift/Gamma/Gain widget publishes one handle per wheel.
        for (const auto &entry : ctx.value(QStringLiteral("colorWheels")).toList()) {
            const QVariantMap d = entry.toMap();
            if (wheelOf(d) == wheel) {
                return d.value(contract::kOptTarget).toString();
            }
        }
    }
    if (wheelControl && !wheel.isEmpty() && path.endsWith(QLatin1String(".target"))) {
        const QVariantMap d = valueAtPath(ctx, path.chopped(7)).toMap();
        if (!wheelOf(d).isEmpty() && wheelOf(d) != wheel) {
            return {};  // e.g. the focused or hovered wheel is a different one
        }
    }
    return valueAtPath(ctx, path).toString();
}

void Engine::handle(const PadEvent &e)
{
    if (kdenliveProfile() && m_kd && !m_kd->isAvailable() && !m_kd->isAbsent()) {
        m_kd->retry();  // not answered definitively yet: ask again (rate-limited by the client)
    }
    switch (e.type) {
    case PadEvent::KeyUp:
    case PadEvent::PressUp: {
        if (m_cheatsheetHold.remove(e.control)) {
            Q_EMIT cheatsheetRequested(QStringLiteral("hide"));
        }
        const bool wasDown = m_down.remove(e.control);
        // A tap deferred to release fires unless the hold was used: the knob
        // turned (a shift), or another input came while this held-layer key
        // was down. An inferred release (pad gone) never fires one.
        const int deferred = m_deferred.take(e.control);
        const bool turned = m_shiftTurned.remove(e.control);
        const bool used = m_usedWhileHeld.remove(e.control);
        const bool oneAtATime = m_oneAtATime.contains(e.control);
        if (oneAtATime && !e.synthetic) {
            m_lastOneUp = e.control;
            m_lastOneUpAt.start();
        }
        if (wasDown && isHeldModifier(e.control)) {
            Q_EMIT resolutionChanged();  // its held layers end (the cheatsheet follows)
        }
        const bool fire = deferred && !e.synthetic && (!(deferred & DeferShift) || !turned) && (!(deferred & DeferModifier) || !used);
        if (fire) {
            const QString slot = e.type == PadEvent::KeyUp ? e.control : e.control + QStringLiteral(".press");
            if (oneAtATime && (deferred & DeferModifier)) {
                // Maybe not a release: another matrix key going down reports this one up.
                // Resolved now: a key going down meanwhile (key1's held layer)
                // must not change what this tap does.
                m_rollTimer->stop();
                m_rollControl = e.control;
                m_rollTap = resolve(slot);
                m_rollTimer->start();
            } else {
                fireDeferred(slot);
            }
        }
        return;
    }
    case PadEvent::KeyDown:
    case PadEvent::PressDown: {
        bool reReported = false;
        if (m_oneAtATime.contains(e.control)) {
            if (m_rollTimer->isActive() && m_rollControl != e.control) {
                // The key "released" just now was still held: no tap for it.
                m_rollTimer->stop();
                m_rollControl.clear();
                m_rollTap.reset();
            }
            reReported = !m_lastOneUp.isEmpty() && m_lastOneUp != e.control && m_lastOneUpAt.isValid() && m_lastOneUpAt.elapsed() < kRollMs;
        }
        noteUse(e.control);
        const bool key = e.type == PadEvent::KeyDown;
        const QString slot = key ? e.control : e.control + QStringLiteral(".press");
        // Its own binding as it was before it went down: its held layers are
        // not active for itself.
        const auto r = resolve(slot);
        int defer = 0;
        if (!key) {
            m_shiftTurned.remove(e.control);
            const auto shift = resolve(QStringList{e.control + QStringLiteral(".shift.turn"), e.control + QStringLiteral(".shift.cw"),
                                                   e.control + QStringLiteral(".shift.ccw")});
            if (shift && shift->binding.isValid()) {
                defer |= DeferShift;
            }
        }
        const bool modifier = isHeldModifier(e.control);
        const bool holdBinding = r && r->binding.kind == Binding::Cheatsheet && r->binding.name == QLatin1String("hold");
        if (modifier && r && r->binding.isValid() && !holdBinding) {
            defer |= DeferModifier;  // a tap on a held-layer key: on release, if nothing else was used
        }
        m_down.insert(e.control);
        m_usedWhileHeld.remove(e.control);
        if (reReported) {
            m_usedWhileHeld.insert(e.control);  // still held across another key's press: not a new tap
        }
        if (modifier) {
            Q_EMIT resolutionChanged();  // its held layers start
        }
        if (defer) {
            m_deferred.insert(e.control, defer);
            return;
        }
        m_deferred.remove(e.control);
        if (r) {
            execute(*r, slot, 1, false);
        }
        return;
    }
    case PadEvent::Turn: {
        noteUse(e.control);
        auto &t = m_lastTurn[e.control];
        const bool fast = t.isValid() && t.elapsed() < m_cfg.settings.accelWindowMs;
        t.start();
        QStringList candidates = turnSlots(e.control, e.delta);
        std::optional<Resolution> r;
        if (m_down.contains(e.control)) {
            const QStringList shifted = shiftSlots(e.control, e.delta);
            r = resolve(shifted);
            if (r && r->binding.isValid()) {
                m_shiftTurned.insert(e.control);
                candidates = shifted;
            } else {
                r.reset();
            }
        }
        if (!r) {
            r = resolve(candidates);
        }
        if (!r || !r->binding.isValid()) {
            return;  // unbound, or explicitly "none" at the winning level
        }
        // Per-binding "accel" overrides settings.accelFactor (1 disables it).
        const double accel = !fast ? 1.0 : (r->binding.accel > 0 ? r->binding.accel : m_cfg.settings.accelFactor);
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
    Q_EMIT dispatched(slot, b.describe(), r.layer);
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
            // Not Kdenlive, or stock Kdenlive with keyFallback: the action's stock shortcut.
            if (!b.keys.isEmpty()) {
                enqueueTaps(group, dir, b.keys, isTurn ? count : 1);
            } else if (kd) {
                sayOnce(QStringLiteral("nofallback|") + b.name, QStringLiteral("%1: no stock shortcut configured for %2").arg(slot, b.name));
            }
        } else if (kdenliveAbsent()) {
            noticeAbsent(b.name);
        } else if (kdenliveActive()) {
            if (!m_kd->supportsAction(b.name)) {
                sayOnce(QStringLiteral("action|") + b.name, QStringLiteral("Kdenlive does not offer action %1 to control surfaces").arg(b.name));
                return;
            }
            endAllGestures(false);  // discrete operations close editing gestures first
            m_actionFallback.insert(b.name, Fallback{m_profile->keyFallback ? b.keys : QList<KeyChord>{}, m_window.pid, m_window.address});
            for (int i = 0; i < (isTurn ? count : 1); ++i) {
                m_kd->triggerAction(b.name);
            }
        } else {
            sayOnce(QStringLiteral("pending"), QStringLiteral("Kdenlive has not answered yet; %1 dropped (no keyboard fallback)").arg(b.name));
        }
        return;
    case Binding::Control: {
        Binding cb = b;
        if (cb.name.startsWith(QLatin1Char('$'))) {
            cb.name = modeValue(cb.name.mid(1));  // a mode selects which control the knob drives
            if (cb.name.isEmpty()) {
                sayOnce(QStringLiteral("mode|") + b.name, QStringLiteral("%1: unknown mode %2").arg(slot, b.name));
                return;
            }
        }
        executeControl(cb, slot, group, dir, (isTurn ? detents : 1) * accel * b.scale);
        return;
    }
    case Binding::Command:
        Q_EMIT runCommand(b.argv);
        return;
    case Binding::Mouse:
        enqueueMouse(group, dir, b.name, count);  // wheels: one detent per knob detent
        return;
    case Binding::Cheatsheet:
        if (b.name == QLatin1String("hold") && !isTurn && !m_inRelease) {
            m_cheatsheetHold.insert(group);  // hidden again when this control comes up
            Q_EMIT cheatsheetRequested(QStringLiteral("show"));
        } else {
            Q_EMIT cheatsheetRequested(QStringLiteral("toggle"));
        }
        return;
    case Binding::Cycle:
    case Binding::Mode: {
        QString owner;
        if (!modesFor(b.name, &owner)) {
            say(QStringLiteral("%1: unknown mode %2").arg(b.kind == Binding::Cycle ? QStringLiteral("cycle") : QStringLiteral("mode"), b.name));
            return;
        }
        endAllGestures(false);  // options change: the next turn is a new gesture
        if (b.kind == Binding::Cycle) {
            stepMode(b.name, b.step);
        } else {
            setModeValue(b.name, b.value);
        }
        Q_EMIT resolutionChanged();  // layers and labels may follow the mode
        announceMode(b.name, b.label, b.notify);
        applyAutoModes();
        return;
    }
    case Binding::Sequence: {
        // Each step as if it were the binding (same slot and detents); with a
        // delay, later steps wait in order (dropped if the window changes).
        // While steps wait, a new sequence queues behind them, so sequences
        // never interleave. A knob turning back drops what it queued the
        // other way, as typed taps do.
        if (isTurn) {
            const auto removed = m_steps.removeIf([&](const PendingStep &p) {
                return p.isTurn && p.slot.section(QLatin1Char('.'), 0, 0) == group && p.direction != dir;
            });
            if (removed) {
                m_stepTimer->stop();  // restarted below with the new first step's own delay
            }
        }
        // All or nothing: a sequence is never cut in half by the limit.
        const bool runFirstNow = m_steps.isEmpty();
        const qsizetype queued = qsizetype(b.steps.size()) - (runFirstNow ? 1 : 0);
        if (m_steps.size() + queued > kMaxPendingSteps) {
            sayOnce(QStringLiteral("steps"), QStringLiteral("%1: too many sequence steps waiting; this one dropped").arg(slot));
            return;
        }
        bool first = true;
        for (const Binding &s : b.steps) {
            Resolution sr = r;
            sr.binding = s;
            if (first && runFirstNow) {
                execute(sr, slot, detents, isTurn, accel);
            } else {
                m_steps << PendingStep{sr, slot, detents, isTurn, accel, first ? 0 : b.delayMs, dir};
            }
            first = false;
        }
        if (!m_steps.isEmpty() && !m_stepTimer->isActive()) {
            m_stepTimer->start(m_steps.first().delayMs);
        }
        return;
    }
    case Binding::Request:
        if (kdenliveActive()) {
            if (!m_kd->supportsCommand(b.name)) {
                sayOnce(QStringLiteral("command|") + b.name, QStringLiteral("Kdenlive does not offer command %1").arg(b.name));
                return;
            }
            QString missing;
            QVariantMap args = expandOptions(b.options, &missing);
            if (!missing.isEmpty()) {
                sayOnce(QStringLiteral("ctx|") + b.name + missing, QStringLiteral("%1: %2 is not in Kdenlive's context; nothing sent").arg(b.name, missing));
                return;
            }
            const QString path = b.targetFrom.isEmpty() ? defaultTargetPath(b.name) : b.targetFrom;
            if (!path.isEmpty() && !args.contains(contract::kOptTarget)) {
                const QString target = resolveTarget(b, b.name, args);
                if (target.isEmpty()) {
                    sayOnce(QStringLiteral("notarget|") + b.name, QStringLiteral("%1: no %2 in Kdenlive's context").arg(b.name, path));
                    return;
                }
                args.insert(contract::kOptTarget, target);
            }
            endAllGestures(false);
            m_kd->invoke(b.name, args);
        } else if (kdenliveStock()) {
            if (!b.keys.isEmpty()) {
                enqueueTaps(group, dir, b.keys, 1);
            }
        } else if (kdenliveAbsent()) {
            noticeAbsent(b.name);
        } else {
            sayOnce(QStringLiteral("pending"), QStringLiteral("Kdenlive has not answered yet; %1 dropped (no keyboard fallback)").arg(b.name));
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
    if (kdenliveAbsent()) {
        noticeAbsent(b.name);
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
    QString missing;
    const QVariantMap opts = expandOptions(b.options, &missing);
    if (!missing.isEmpty()) {
        sayOnce(QStringLiteral("ctx|") + b.name + missing, QStringLiteral("%1: %2 is not in Kdenlive's context; nothing sent").arg(b.name, missing));
        return;
    }
    QString key;
    QVariantMap payloadOptions = opts;
    if (!contract::isEditingControl(b.name)) {
        // Kdenlive ends the editing gesture when any other control applies; end
        // ours first so a later turn starts a fresh gesture id.
        endAllGestures(false);
    }
    if (contract::isEditingControl(b.name)) {
        const QString bindingId = slot + QLatin1Char('|') + b.name + QLatin1Char('|') + optionsKey(opts);
        const quint64 epoch = m_kd->epoch();
        auto it = m_gestures.find(bindingId);
        if (it != m_gestures.end() && (it->epoch != epoch || it->last.elapsed() > m_cfg.settings.gestureIdleMs)) {
            endGesture(bindingId, false);
            it = m_gestures.end();
        }
        if (it != m_gestures.end() && b.name == contract::kTrim && it->batches >= m_kd->limit(QStringLiteral("trimGestureSteps"), 128) - 1) {
            // The host retains every resize step of a gesture: end this one (its
            // end barrier may carry one last step) before starting a new one.
            endGesture(bindingId, false);
            it = m_gestures.end();
        }
        // An open gesture keeps the target it captured; only a new gesture reads
        // the context (so a moving hover can never retarget a turn in progress).
        const QString target = it != m_gestures.end() ? it->target : resolveTarget(b, b.name, opts);
        if (target.isEmpty()) {
            sayOnce(QStringLiteral("notarget|") + b.name, QStringLiteral("%1: no editing target in Kdenlive's context").arg(b.name));
            return;
        }
        if (it == m_gestures.end()) {
            // Kdenlive keeps one active editing gesture per host: close ours
            // (end barriers go out first) before another binding starts one.
            endAllGestures(false);
            Gesture g;
            g.id = QStringLiteral("cs-%1-%2").arg(QCoreApplication::applicationPid()).arg(++m_gestureCounter);
            g.key = bindingId + QStringLiteral("|") + g.id + QStringLiteral("|") + target;
            g.control = b.name;
            g.target = target;
            g.epoch = epoch;
            g.options = opts;
            const QVariantMap ctx = m_kd->context();
            const QVariantMap desc = descriptorFor(ctx, target);
            g.frameBound = desc.contains(QStringLiteral("liveGrading")) ? !desc.value(QStringLiteral("liveGrading")).toBool()
                                                                        : desc.value(QStringLiteral("frame"), -1).toInt() >= 0;
            g.position = ctx.value(QStringLiteral("position"));
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
    if (!kdenliveActive()) {
        return;
    }
    const QVariantMap options = payload.value(QStringLiteral("options")).toMap();
    if (!m_kd->control(key, payload.value(QStringLiteral("name")).toString(), delta, options)) {
        m_coalescer.ack(key);  // not sent: do not wait for an ack that cannot come
        return;
    }
    const QString control = payload.value(QStringLiteral("name")).toString();
    if (control == contract::kTrackFocus || control == contract::kParamFocus) {
        m_navInFlight.insert(key, m_kd->epoch());
    }
    if (!isEnd && options.contains(contract::kOptGesture)) {
        for (auto it = m_gestures.begin(); it != m_gestures.end(); ++it) {
            if (it->key == key) {
                ++it->batches;
                break;
            }
        }
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
            m_tapQueue.append(Tap{group, dir, c, QString()});
        }
    }
    if (!m_tapTimer->isActive()) {
        // First tap goes out at once; the timer then acts as a cooldown so later
        // taps are spaced by 1/keyRateHz.
        drainTap();
        m_tapTimer->start();
    }
}

void Engine::enqueueMouse(const QString &group, int dir, const QString &action, int count)
{
    if (dir != 0) {
        m_tapQueue.erase(std::remove_if(m_tapQueue.begin(), m_tapQueue.end(),
                                        [&](const Tap &t) { return t.group == group && t.dir == -dir; }),
                         m_tapQueue.end());
    }
    for (int i = 0; i < count && m_tapQueue.size() < kMaxQueuedTaps; ++i) {
        m_tapQueue.append(Tap{group, dir, KeyChord{}, action});
    }
    if (!m_tapTimer->isActive()) {
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
    if (!t.mouse.isEmpty()) {
        m_keys->mouse(t.mouse);
    } else {
        m_keys->tap(t.chord);
    }
}

} // namespace cs
