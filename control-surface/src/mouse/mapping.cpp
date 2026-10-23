// SPDX-License-Identifier: GPL-2.0-or-later
#include "mapping.h"
#include "installed.h"
#include <QJsonArray>
#include <QJsonDocument>
#include <linux/input.h>

namespace cs::mouse {
namespace {
std::optional<Mapping> fail(QString *error, const QString &message) {
    *error = message;
    return std::nullopt;
}

bool validSlot(const QString &slot) {
    static const QRegularExpression pattern(QStringLiteral("^(grid-([1-9]|1[0-2])|btn-(left|right|middle|back|"
                                                           "forward|dpi|dpi-up|dpi-down|scroll-mode)|wheel-(up|down|tilt-left|tilt-right))$"));
    return pattern.match(slot).hasMatch();
}

bool validateBinding(const Binding &binding, bool kdenlive, QString *error) {
    if ((binding.kind == Binding::Action || binding.kind == Binding::Control || binding.kind == Binding::Request) && !kdenlive) {
        *error = QStringLiteral("enable Kdenlive API for this app profile");
        return false;
    }
    if (binding.kind == Binding::Cheatsheet) {
        *error = QStringLiteral("keypad cheatsheet actions are not supported on a mouse");
        return false;
    }
    for (const auto &step : binding.steps)
        if (!validateBinding(step, kdenlive, error))
            return false;
    return true;
}

bool validInput(const QString &slot, const SourceKey &input) {
    if (!validSlot(slot) || input.source.isEmpty())
        return false;
    if (input.type == EV_REL)
        return (input.direction == 1 || input.direction == -1) &&
               (input.code == REL_WHEEL || input.code == REL_HWHEEL) &&
               Mapping::wheelSlot(input.code, input.direction) == slot;
    if (input.type != EV_KEY || input.direction != 0 || input.code <= 0 || input.code > KEY_MAX || modifierBit(input.code))
        return false;
    const int primary = slot == "btn-left" ? BTN_LEFT : slot == "btn-right" ? BTN_RIGHT : slot == "btn-middle" ? BTN_MIDDLE : 0;
    if (primary)
        return input.code == primary;
    return input.code != BTN_LEFT && input.code != BTN_RIGHT && input.code != BTN_MIDDLE;
}
} // namespace

SourceKey SourceKey::relative(const QString &source, int code, int value) {
    if (code == REL_WHEEL_HI_RES)
        code = REL_WHEEL;
    else if (code == REL_HWHEEL_HI_RES)
        code = REL_HWHEEL;
    return {source, code, EV_REL, value > 0 ? 1 : -1};
}

QStringList SetupFlow::steps() {
    QStringList result{"btn-left", "btn-right", "btn-middle", "wheel-up", "wheel-down", "wheel-tilt-left", "wheel-tilt-right"};
    for (int number = 1; number <= 12; ++number)
        result << QStringLiteral("grid-%1").arg(number);
    return result << "btn-dpi-up" << "btn-dpi-down" << "btn-scroll-mode";
}

void SetupFlow::start() {
    m_inputs.clear();
    m_index = 0;
    m_armed = false;
}

void SetupFlow::cancel() {
    m_inputs.clear();
    m_index = -1;
    m_armed = false;
}

bool SetupFlow::arm(QString *error) {
    if (!active() || review()) {
        *error = QStringLiteral("Setup is not waiting for input");
        return false;
    }
    m_inputs.remove(target());
    m_armed = true;
    return true;
}

bool SetupFlow::capture(const SourceKey &input, QString *error) {
    if (!armed() || target().isEmpty()) {
        *error = QStringLiteral("Click Capture next press before pressing the highlighted control");
        return false;
    }
    if (!validInput(target(), input)) {
        *error = QStringLiteral("That input does not match %1. Primary clicks and wheel directions must keep their safe identity; "
                                "use a single non-modifier key for other buttons. Try again or Skip.")
                     .arg(target());
        return false;
    }
    for (auto it = m_inputs.begin(); it != m_inputs.end(); ++it)
        if (*it == input) {
            *error = QStringLiteral("This input already belongs to %1. Try again or Skip; nothing saved.").arg(it.key());
            return false;
        }
    m_inputs.insert(target(), input);
    m_armed = false;
    return true;
}

bool SetupFlow::next(bool skip, QString *error) {
    if (!active() || review() || (!skip && !captured())) {
        *error = QStringLiteral("Capture the highlighted control or choose Skip");
        return false;
    }
    if (skip)
        m_inputs.remove(target());
    ++m_index;
    m_armed = false;
    return true;
}

void SetupFlow::retry() {
    if (active() && !review()) {
        m_inputs.remove(target());
        m_armed = false;
    }
}

bool GridCalibration::capture(const SourceKey &key, QString *error) {
    if (!active() || complete()) {
        *error = QStringLiteral("Calibration is not waiting for another button");
        return false;
    }
    if (key.type != EV_KEY || key.direction != 0 || key.source.isEmpty() || key.code <= 0 || key.code > KEY_MAX ||
        (key.code >= BTN_LEFT && key.code <= BTN_MIDDLE) ||
        modifierBit(key.code)) {
        *error = QStringLiteral("Press a numbered side button, not a primary click or modifier");
        return false;
    }
    for (auto it = m_inputs.begin(); it != m_inputs.end(); ++it)
        if (it->source == key.source && it->code == key.code) {
            *error = QStringLiteral("This input was already captured for %1. Press %2, or cancel and restart.").arg(it.key(), target());
            return false;
        }
    m_inputs.insert(target(), key);
    return true;
}

std::optional<Mapping> Mapping::parse(const QByteArray &text, QString *error) {
    QJsonParseError jsonError;
    const auto doc = QJsonDocument::fromJson(stripJsonComments(text), &jsonError);
    if (!doc.isObject())
        return fail(error, jsonError.error == QJsonParseError::NoError ? QStringLiteral("mouse.json: root must be a JSON object")
                                                                       : QStringLiteral("mouse.json: %1").arg(jsonError.errorString()));
    const auto root = doc.object();
    if (root.contains("version") && root.value("version").toDouble(-1) != 1)
        return fail(error, QStringLiteral("mouse.json: unsupported version"));
    if (root.contains("devices") && !root.value("devices").isObject())
        return fail(error, QStringLiteral("mouse.json: devices must be an object"));
    const auto devices = root.value("devices").toObject();
    if (devices.contains("mmo-12") && !devices.value("mmo-12").isObject())
        return fail(error, QStringLiteral("mouse.json: mmo-12 must be an object"));
    const auto device = devices.value("mmo-12").toObject();
    Mapping result;
    if (device.contains("profiles") && !device.value("profiles").isArray())
        return fail(error, QStringLiteral("mmo-12: profiles must be an array"));
    for (const auto &entry : device.value("profiles").toArray()) {
        if (!entry.isObject())
            return fail(error, QStringLiteral("mmo-12: profile must be an object"));
        const auto object = entry.toObject();
        const QString name = object.value("name").toString();
        if (object.contains("match")) {
            if (!object.value("match").isObject())
                return fail(error, QStringLiteral("%1: match must be an object").arg(name));
            const auto match = object.value("match").toObject();
            for (auto it = match.begin(); it != match.end(); ++it)
                if (it.key() != "class" && it.key() != "title")
                    return fail(error, QStringLiteral("%1: unknown match field %2").arg(name, it.key()));
            for (const auto &field : {QStringLiteral("class"), QStringLiteral("title")}) {
                if (match.contains(field) && !match.value(field).isString())
                    return fail(error, QStringLiteral("%1: match.%2 must be a string").arg(name, field));
            }
        }
        if (object.contains("fallthrough") && !object.value("fallthrough").isBool())
            return fail(error, QStringLiteral("%1: fallthrough must be boolean").arg(name));
        if (object.contains("bindings") && !object.value("bindings").isObject())
            return fail(error, QStringLiteral("%1: bindings must be an object").arg(name));
        const auto bindings = object.value("bindings").toObject();
        for (auto it = bindings.begin(); it != bindings.end(); ++it) {
            if (!validSlot(it.key()))
                return fail(error, QStringLiteral("%1 / %2: unknown mouse slot").arg(name, it.key()));
            QString reason;
            auto binding = parseBinding(it.value(), &reason);
            if (!binding)
                return fail(error, QStringLiteral("%1 / %2: %3").arg(name, it.key(), reason));
            if ((it.key() == "btn-left" || it.key() == "btn-right") &&
                (binding->kind != Binding::Mouse || (binding->name != "left" && binding->name != "right")))
                return fail(error, QStringLiteral("%1: primary clicks may only map to left/right clicks").arg(it.key()));
        }
    }
    QStringList slotsList;
    for (int number = 1; number <= 12; ++number)
        slotsList << QStringLiteral("grid-%1").arg(number);
    slotsList << "btn-left" << "btn-right" << "btn-middle" << "btn-back" << "btn-forward" << "btn-dpi"
              << "btn-dpi-up" << "btn-dpi-down" << "btn-scroll-mode"
              << "wheel-tilt-left" << "wheel-tilt-right" << "wheel-up" << "wheel-down";
    const auto shared = parseConfig(QJsonDocument(QJsonObject{{"profiles", device.value("profiles")}}).toJson(), {}, error, slotsList);
    if (!shared)
        return std::nullopt;
    result.profiles = shared->profiles;
    // A mouse API assignment must never become an implicit stock shortcut.
    for (const auto &profile : result.profiles) {
        if (profile.keyFallback)
            return fail(error, QStringLiteral("%1: mouse API actions do not support keyFallback").arg(profile.name));
        if (profile.kdenlive && !profile.hasMatch)
            return fail(error, QStringLiteral("%1: Kdenlive API needs an app-matched profile, not Global").arg(profile.name));
        for (auto it = profile.bindings.begin(); it != profile.bindings.end(); ++it) {
            QString reason;
            if (!validateBinding(*it, profile.kdenlive, &reason))
                return fail(error, QStringLiteral("%1 / %2: %3").arg(profile.name, it.key(), reason));
        }
        for (const auto &layer : profile.layers) {
            if (!layer.held.isEmpty())
                return fail(
                    error,
                    QStringLiteral("%1 / %2: keypad held-control layers are not supported on a mouse").arg(profile.name, layer.name));
            for (auto it = layer.bindings.begin(); it != layer.bindings.end(); ++it) {
                QString reason;
                if (!validateBinding(*it, profile.kdenlive, &reason))
                    return fail(error, QStringLiteral("%1 / %2 / %3: %4").arg(profile.name, layer.name, it.key(), reason));
            }
            for (auto it = layer.bindings.begin(); it != layer.bindings.end(); ++it)
                if ((it.key() == "btn-left" || it.key() == "btn-right") &&
                    (it->kind != Binding::Mouse || (it->name != "left" && it->name != "right")))
                    return fail(error, QStringLiteral("%1: primary clicks may only map to left/right clicks").arg(it.key()));
        }
    }
    if (device.contains("learned") && !device.value("learned").isObject())
        return fail(error, QStringLiteral("mmo-12: learned must be an object"));
    const auto learned = device.value("learned").toObject();
    for (auto it = learned.begin(); it != learned.end(); ++it) {
        const auto object = it.value().toObject();
        const QString source = object.value("source").toString();
        const auto value = object.value("code");
        const int code = value.toInt(-1);
        const int type = object.contains("type") ? object.value("type").toInt(-1) : EV_KEY;
        const int direction = object.contains("direction") ? object.value("direction").toInt(0) : 0;
        const SourceKey key{source, code, type, direction};
        if (!value.isDouble() || value.toDouble() != code ||
            (object.contains("type") && object.value("type").toDouble(-1) != type) ||
            (object.contains("direction") && object.value("direction").toDouble(0.5) != direction) || !validInput(it.key(), key))
            return fail(error, QStringLiteral("mmo-12 / learned / %1: invalid source or key code").arg(it.key()));
        if (result.learned.contains(key))
            return fail(error, QStringLiteral("mmo-12: two slots name the same learned input"));
        result.learned.insert(key, it.key());
    }
    return result;
}

std::optional<Binding> Mapping::binding(const QString &slot, const WindowInfo &window, bool focusAvailable) const {
    RecordingKeySink sink;
    Engine engine(&sink, nullptr);
    auto cfg = config();
    if (!focusAvailable)
        cfg.profiles.removeIf([](const Profile &profile) { return profile.hasMatch; });
    engine.setConfig(cfg);
    engine.setActiveWindow(window);
    if (const auto resolved = engine.resolve(slot))
        return resolved->binding;
    return std::nullopt;
}

QString Mapping::slot(const SourceKey &key, bool pointer) const {
    Q_UNUSED(pointer);
    if (learned.contains(key))
        return learned.value(key);
    switch (key.code) {
    case BTN_LEFT:
        return QStringLiteral("btn-left");
    case BTN_RIGHT:
        return QStringLiteral("btn-right");
    case BTN_MIDDLE:
        return QStringLiteral("btn-middle");
    case BTN_SIDE:
    case BTN_BACK:
        return QStringLiteral("btn-back");
    case BTN_EXTRA:
    case BTN_FORWARD:
        return QStringLiteral("btn-forward");
    default:
        return {};
    }
}

QString Mapping::wheelSlot(int axis, int direction) {
    if (axis == REL_WHEEL || axis == REL_WHEEL_HI_RES)
        return direction > 0 ? QStringLiteral("wheel-up") : QStringLiteral("wheel-down");
    if (axis == REL_HWHEEL || axis == REL_HWHEEL_HI_RES)
        return direction > 0 ? QStringLiteral("wheel-tilt-right") : QStringLiteral("wheel-tilt-left");
    return {};
}

QString Mapping::relativeSlot(const QString &source, int code, int value) const {
    const auto input = SourceKey::relative(source, code, value);
    return learned.contains(input) ? learned.value(input) : wheelSlot(code, value);
}

QStringList Mapping::unlearned() const {
    QSet<QString> pending;
    for (const auto &profile : profiles) {
        for (auto it = profile.bindings.begin(); it != profile.bindings.end(); ++it) {
            if (it.key().startsWith("grid-") || it.key().startsWith("btn-dpi") || it.key() == "btn-scroll-mode")
                pending.insert(it.key());
        }
    }
    for (const auto &slot : learned)
        pending.remove(slot);
    auto result = pending.values();
    result.sort();
    return result;
}

void Router::send(bool pointer, int type, int code, int value) {
    output(pointer, {type, code, value});
    output(pointer, {EV_SYN, SYN_REPORT, 0});
}

Router::Router(KdenliveClient *client) : m_engine(this, client) {
    m_engine.setApiFallbackAllowed(false);
    QObject::connect(&m_engine, &Engine::runCommand, this, [this](const QStringList &argv) {
        Binding binding;
        binding.kind = Binding::Command;
        binding.argv = argv;
        if (command)
            command(binding);
    });
}

void Router::setMapping(const Mapping &value) {
    releaseAll();
    mapping = value;
    applyMapping();
}

void Router::applyMapping() {
    auto cfg = mapping.config();
    if (!m_focusAvailable)
        cfg.profiles.removeIf([](const Profile &profile) { return profile.hasMatch; });
    m_engine.setConfig(cfg);
}

void Router::setWindow(const WindowInfo &window, bool focusAvailable) {
    const auto &before = m_engine.activeWindow();
    if (m_focusAvailable != focusAvailable || before.cls != window.cls || before.pid != window.pid || before.address != window.address ||
        m_engine.activeProfile() != m_engine.config().profileFor(window.cls, window.title))
        releaseAll();
    if (m_focusAvailable != focusAvailable) {
        m_focusAvailable = focusAvailable;
        applyMapping();
    }
    m_engine.setActiveWindow(window);
}

void Router::tap(const KeyChord &chord) {
    Held held;
    for (quint8 bit : {Mod::Ctrl, Mod::Shift, Mod::Alt, Mod::Meta})
        if (chord.mods & bit)
            acquire(held, false, modifierKey(bit));
    acquire(held, false, chord.key);
    release(held);
}

void Router::mouse(const QString &action) {
    if (auto events = mouseEvents(action))
        for (const auto &event : *events)
            output(true, event);
}

void Router::acquire(Held &held, bool pointer, int code) {
    const auto key = qMakePair(pointer, code);
    if (m_refs[key]++ == 0)
        send(pointer, EV_KEY, code, 1);
    held.codes.append(key);
}

void Router::release(Held &held) {
    for (auto it = held.codes.crbegin(); it != held.codes.crend(); ++it) {
        if (--m_refs[*it] == 0) {
            send(it->first, EV_KEY, it->second, 0);
            m_refs.remove(*it);
        }
    }
    held.codes.clear();
}

bool Router::key(const SourceKey &key, bool pointer, int value) {
    auto found = m_held.find(key);
    if (value == 0) {
        if (found == m_held.end())
            return false;
        const bool consumed = found->consumed;
        release(*found);
        m_held.erase(found);
        if (released)
            released(key);
        return consumed;
    }
    if (found != m_held.end()) {
        if (value == 2 && identify && key.code != BTN_LEFT && key.code != BTN_RIGHT && key.code != BTN_MIDDLE)
            return true;
        if (value == 2 && found->consumed) {
            for (const auto &code : found->codes)
                if (!modifierBit(code.second))
                    send(code.first, EV_KEY, code.second, 2);
        }
        return found->consumed;
    }
    if (value != 1)
        return false;
    const QString slot = mapping.slot(key, pointer);
    if (pressed)
        pressed(slot, key);
    Held held;
    if (identify) {
        // Composite Naga interfaces can expose both pointer and keyboard keys.
        // Only primary clicks may reach the desktop while identifying.
        held.consumed = key.code != BTN_LEFT && key.code != BTN_RIGHT && key.code != BTN_MIDDLE;
    } else if (!slot.isEmpty()) {
        const auto resolution = m_engine.resolve(slot);
        if (resolution) {
            const auto *binding = &resolution->binding;
            held.consumed = true;
            if (binding->kind == Binding::Keys) {
                for (const auto &chord : binding->keys) {
                    for (quint8 bit : {Mod::Ctrl, Mod::Shift, Mod::Alt, Mod::Meta})
                        if (chord.mods & bit)
                            acquire(held, false, modifierKey(bit));
                    acquire(held, false, chord.key);
                    if (binding->keys.size() > 1)
                        release(held);
                }
            } else if (binding->kind == Binding::Mouse) {
                const auto events = mouseEvents(binding->name);
                if (events && !events->isEmpty()) {
                    if (events->first().type == EV_KEY)
                        acquire(held, true, events->first().code);
                    else
                        for (const auto &event : *events)
                            output(true, event);
                }
            } else {
                m_engine.handle({slot, PadEvent::KeyDown, 0, 0});
                m_engine.handle({slot, PadEvent::KeyUp, 0, 0});
            }
        }
    }
    m_held.insert(key, held);
    return held.consumed;
}

bool Router::consume(const QString &source, bool pointer, const InputTriple &event) {
    if (event.type == EV_SYN && event.code == SYN_REPORT) {
        flushWheels(source);
        m_identifiedWheels.remove(source);
    }
    if (event.type == EV_MSC && event.code == MSC_SCAN)
        return true;
    if (event.type == EV_REL)
        return wheel(source, event);
    return event.type == EV_KEY && key({source, event.code}, pointer, event.value);
}

bool Router::wheel(const QString &source, const InputTriple &event) {
    const QString slot = mapping.relativeSlot(source, event.code, event.value);
    if (event.value == 0 || event.code == REL_X || event.code == REL_Y)
        return false;
    if (slot.isEmpty()) {
        if (!identify)
            return false;
        const auto direction = qMakePair(event.code, event.value > 0 ? 1 : -1);
        if (!m_identifiedWheels[source].contains(direction)) {
            m_identifiedWheels[source].insert(direction);
            if (scrolled)
                scrolled({}, source, event);
        }
        return true;
    }
    const bool horizontal = event.code == REL_HWHEEL || event.code == REL_HWHEEL_HI_RES;
    const int axis = horizontal ? REL_HWHEEL : REL_WHEEL;
    if (identify) {
        const auto direction = qMakePair(axis, event.value > 0 ? 1 : -1);
        if (!m_identifiedWheels[source].contains(direction)) {
            m_identifiedWheels[source].insert(direction);
            if (scrolled)
                scrolled(slot, source, event);
        }
        return true;
    }
    if (!m_engine.resolve(slot))
        return false;
    auto &frame = m_wheels[qMakePair(source, axis)];
    if (event.code == REL_WHEEL_HI_RES || event.code == REL_HWHEEL_HI_RES) {
        frame.hasHighResolution = true;
        frame.highResolution += event.value;
    } else {
        frame.legacy += event.value;
    }
    return true;
}

void Router::dispatchPulse(const QString &slot) {
    m_engine.handle({slot, PadEvent::KeyDown, 0, 0});
    m_engine.handle({slot, PadEvent::KeyUp, 0, 0});
}

void Router::flushWheels(const QString &source) {
    const auto frames = m_wheels;
    for (auto it = frames.begin(); it != frames.end(); ++it) {
        if (it.key().first != source)
            continue;
        m_wheels.remove(it.key());
        int steps = it->legacy;
        if (it->hasHighResolution) {
            auto &remaining = m_wheelRemainder[it.key()];
            remaining += it->highResolution;
            steps = remaining / 120;
            remaining %= 120;
        }
        for (int count = 0; count < qAbs(steps); ++count)
            dispatchPulse(mapping.relativeSlot(source, it.key().second, steps));
    }
}

bool Router::hasOtherKeyDown(const SourceKey &key) const {
    for (auto it = m_held.begin(); it != m_held.end(); ++it)
        if (it.key().source != key.source || it.key().code != key.code)
            return true;
    return false;
}

void Router::releaseAll() {
    m_engine.cancelPending();
    m_wheels.clear();
    m_wheelRemainder.clear();
    m_identifiedWheels.clear();
    // Keep consumed inputs suppressed until their physical release; a config,
    // focus or mode change must never leak an unmatched release to the desktop.
    for (auto &held : m_held)
        release(held);
}

void Router::disconnected() {
    releaseAll();
    m_held.clear();
}

} // namespace cs::mouse
