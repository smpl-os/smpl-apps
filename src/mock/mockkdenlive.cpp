// SPDX-License-Identifier: GPL-2.0-or-later
#include "mockkdenlive.h"
#include "kdenlivecontract.h"

#include <QDBusMessage>
#include <QDBusMetaType>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>
#include <cmath>
#include <cstdio>

namespace cs {

namespace {
const double kShuttle[] = {0.0, 1.0, 2.0, 4.0, 5.0, 8.0, 16.0, 60.0};  // Kdenlive jogaction.cpp

struct MockAction {
    const char *id;
    const char *text;
    const char *shortcut;
};
const MockAction kActions[] = {
    {"monitor_play", "Play/Pause", "Space"},
    {"monitor_pause", "Pause", "K"},
    {"monitor_seek_backward", "Rewind", "J"},
    {"monitor_seek_forward", "Forward", "L"},
    {"monitor_seek_backward-one-frame", "Rewind 1 Frame", "Left"},
    {"monitor_seek_forward-one-frame", "Forward 1 Frame", "Right"},
    {"monitor_seek_snap_backward", "Go to Previous Snap Point", "Alt+Left"},
    {"monitor_seek_snap_forward", "Go to Next Snap Point", "Alt+Right"},
    {"mark_in", "Set Zone In", "I"},
    {"mark_out", "Set Zone Out", "O"},
    {"insert_to_in_point", "Insert Clip Zone in Timeline", "V"},
    {"overwrite_to_in_point", "Overwrite Clip Zone in Timeline", "B"},
    {"switch_monitor", "Switch Monitor", "T"},
    {"cut_timeline_clip", "Cut Clip", "Shift+R"},
    {"delete_timeline_clip", "Delete Selected Item", "Del"},
    {"add_marker_guide_quickly", "Add Marker/Guide quickly", "Num+*"},
    {"select_tool", "Selection Tool", "S"},
    {"razor_tool", "Razor Tool", "X"},
    {"ripple_tool", "Ripple Tool", ""},
    {"roll_tool", "Roll Tool", ""},
    {"slip_tool", "Slip Tool", ""},
    {"slide_tool", "Slide Tool", ""},
    {"keyframe_add", "Add/Remove Keyframe", ""},
    {"keyframe_next", "Go to Next Keyframe", ""},
    {"keyframe_previous", "Go to Previous Keyframe", ""},
    {"zoom_fit", "Fit Zoom to Project", ""},
    {"view_zoom_in", "Zoom In", "Ctrl++"},
    {"view_zoom_out", "Zoom Out", "Ctrl+-"},
    {"edit_undo", "Undo", "Ctrl+Z"},
    {"edit_redo", "Redo", "Ctrl+Shift+Z"},
    {"automation_editor", "Automation Editor", ""},
};

QVariantMap rgb(double v)
{
    return {{QStringLiteral("r"), v}, {QStringLiteral("g"), v}, {QStringLiteral("b"), v}};
}

QString compact(const QVariantMap &m)
{
    return QString::fromUtf8(QJsonDocument(QJsonObject::fromVariantMap(m)).toJson(QJsonDocument::Compact));
}
} // namespace

MockKdenlive::MockKdenlive(QObject *parent)
    : QObject(parent)
{
    qDBusRegisterMetaType<QList<QVariantMap>>();
    m_contextTimer = new QTimer(this);
    m_contextTimer->setSingleShot(true);
    m_contextTimer->setInterval(33);  // <= 30 context signals per second
    connect(m_contextTimer, &QTimer::timeout, this, [this] {
        if (m_contextDirty) {
            emitContextSoon();
        }
    });
    m_wheels = {{QStringLiteral("lift"), rgb(0.0)}, {QStringLiteral("gamma"), rgb(1.0)}, {QStringLiteral("gain"), rgb(1.0)}};
    m_paramOrder = {QStringLiteral("level"), QStringLiteral("opacity")};
    m_params = {{QStringLiteral("level"), 50.0}, {QStringLiteral("opacity"), 100.0}};
    m_curve = {{QStringLiteral("value"), 0.0}, {QStringLiteral("time"), 0}};
    m_context = {{QStringLiteral("focus"), QStringLiteral("timeline")},
                 {QStringLiteral("activeMonitor"), QStringLiteral("project")},
                 {QStringLiteral("tool"), QStringLiteral("select")},
                 {QStringLiteral("playing"), false},
                 {QStringLiteral("position"), 0},
                 {QStringLiteral("fps"), 25.0}};
}

bool MockKdenlive::registerOn(QDBusConnection connection)
{
    return connection.registerObject(contract::kPath, this, QDBusConnection::ExportAllSlots | QDBusConnection::ExportAllSignals);
}

void MockKdenlive::record(const QString &line)
{
    log << line;
    if (m_print) {
        std::printf("%s\n", qPrintable(line));
        std::fflush(stdout);
    }
}

void MockKdenlive::setContext(const QVariantMap &context)
{
    m_context = context;
    emitContextSoon();
}

void MockKdenlive::setContextValue(const QString &key, const QVariant &value)
{
    if (value.isValid()) {
        m_context.insert(key, value);
    } else {
        m_context.remove(key);
    }
    emitContextSoon();
}

void MockKdenlive::emitContextSoon()
{
    m_context.insert(QStringLiteral("serial"), ++m_serial);
    if (m_subscribers.isEmpty()) {
        m_contextDirty = false;  // no client: no signal traffic at all
        return;
    }
    if (m_contextTimer->isActive()) {
        m_contextDirty = true;
        return;
    }
    m_contextDirty = false;
    ++m_contextSignals;
    Q_EMIT ContextChanged(m_context);
    m_contextTimer->start();
}

QVariantMap MockKdenlive::state() const
{
    return {{QStringLiteral("position"), m_position},
            {QStringLiteral("shuttle"), m_shuttle},
            {QStringLiteral("speed"), (m_shuttle < 0 ? -1 : 1) * kShuttle[std::abs(m_shuttle)]},
            {QStringLiteral("zoom"), m_zoom},
            {QStringLiteral("scroll"), m_scroll},
            {QStringLiteral("track"), m_track},
            {QStringLiteral("trim"), m_trim},
            {QStringLiteral("gainDb"), m_gainDb},
            {QStringLiteral("wheels"), m_wheels},
            {QStringLiteral("params"), m_params},
            {QStringLiteral("curve"), m_curve},
            {QStringLiteral("triggered"), m_triggered}};
}

QVariantMap MockKdenlive::Capabilities()
{
    return {{QStringLiteral("version"), contract::kVersion},
            {QStringLiteral("controls"), QStringList{contract::kJog, contract::kShuttle, contract::kZoom, contract::kScroll, contract::kTrackFocus,
                                                     contract::kParamNudge, QStringLiteral("param.focus"), contract::kColorWheel, contract::kCurveNudge,
                                                     contract::kTrim, contract::kAudioGain}},
            {QStringLiteral("commands"), QStringList{QStringLiteral("colorwheel.reset"), QStringLiteral("param.reset")}},
            {QStringLiteral("contextKeys"), QStringList{QStringLiteral("focus"), QStringLiteral("activeMonitor"), QStringLiteral("tool"),
                                                        QStringLiteral("playing"), QStringLiteral("position"), QStringLiteral("effect"),
                                                        QStringLiteral("param"), QStringLiteral("colorWheel"), QStringLiteral("automation")}},
            {QStringLiteral("implementation"), QStringLiteral("control-surface mock")}};
}

QVariantMap MockKdenlive::Subscribe()
{
    const QString who = calledFromDBus() ? message().service() : QStringLiteral("local");
    m_subscribers.insert(who.isEmpty() ? QStringLiteral("peer") : who);
    record(QStringLiteral("Subscribe from %1").arg(who.isEmpty() ? QStringLiteral("peer") : who));
    m_context.insert(QStringLiteral("serial"), ++m_serial);
    return m_context;
}

void MockKdenlive::Unsubscribe()
{
    const QString who = calledFromDBus() ? message().service() : QStringLiteral("local");
    m_subscribers.remove(who.isEmpty() ? QStringLiteral("peer") : who);
    record(QStringLiteral("Unsubscribe"));
}

QVariantMap MockKdenlive::GetContext()
{
    return m_context;
}

QList<QVariantMap> MockKdenlive::ListActions()
{
    QList<QVariantMap> out;
    for (const auto &a : kActions) {
        out << QVariantMap{{QStringLiteral("id"), QString::fromLatin1(a.id)},
                           {QStringLiteral("text"), QString::fromLatin1(a.text)},
                           {QStringLiteral("shortcut"), QString::fromLatin1(a.shortcut)},
                           {QStringLiteral("enabled"), true},
                           {QStringLiteral("checkable"), QString::fromLatin1(a.id).endsWith(QLatin1String("_tool"))}};
    }
    return out;
}

bool MockKdenlive::TriggerAction(const QString &id)
{
    for (const auto &a : kActions) {
        if (id == QLatin1String(a.id)) {
            m_triggered << id;
            record(QStringLiteral("TriggerAction %1").arg(id));
            if (id.endsWith(QLatin1String("_tool"))) {
                setContextValue(QStringLiteral("tool"), id.chopped(5));
            } else if (id == QLatin1String("monitor_play")) {
                setContextValue(QStringLiteral("playing"), !m_context.value(QStringLiteral("playing")).toBool());
            }
            return true;
        }
    }
    record(QStringLiteral("TriggerAction %1 (unknown)").arg(id));
    return false;
}

void MockKdenlive::Control(const QString &control, double delta, const QVariantMap &options, uint seq)
{
    ++m_controlMessages;
    const QString key = control + QLatin1Char('|') + compact(options);
    Pending &p = m_pending[key];
    if (p.control.isEmpty()) {
        m_pendingOrder << key;
    }
    p.control = control;
    p.options = options;
    p.delta += delta;
    p.seq = seq;
    if (!m_applyScheduled) {
        // Apply once per event-loop pass (or after the simulated GUI cost),
        // merging whatever arrived meanwhile.
        m_applyScheduled = true;
        QTimer::singleShot(m_applyDelayMs, this, &MockKdenlive::applyPending);
    }
}

void MockKdenlive::applyPending()
{
    m_applyScheduled = false;
    ++m_applyBatches;
    const QStringList order = m_pendingOrder;
    m_pendingOrder.clear();
    for (const auto &key : order) {
        const Pending p = m_pending.take(key);
        const QVariantMap st = applyControl(p.control, p.delta, p.options);
        record(QStringLiteral("Control %1 %2 %3 -> %4").arg(p.control).arg(p.delta).arg(compact(p.options), compact(st)));
        Q_EMIT ControlAck(p.seq, p.control, st);
    }
}

QVariantMap MockKdenlive::applyControl(const QString &control, double delta, const QVariantMap &options)
{
    const int steps = int(std::lround(delta));
    if (control == contract::kJog) {
        m_position = qBound(0, m_position + steps, m_duration);
        m_context.insert(QStringLiteral("position"), m_position);
        return {{QStringLiteral("position"), m_position}};
    }
    if (control == contract::kShuttle) {
        m_shuttle = qBound(-7, m_shuttle + steps, 7);
        const double speed = (m_shuttle < 0 ? -1 : 1) * kShuttle[std::abs(m_shuttle)];
        m_context.insert(QStringLiteral("playing"), m_shuttle != 0);
        return {{QStringLiteral("shuttle"), m_shuttle}, {QStringLiteral("speed"), speed}};
    }
    if (control == contract::kZoom) {
        m_zoom = qBound(0.0, m_zoom + delta, 20.0);
        return {{QStringLiteral("zoom"), m_zoom}};
    }
    if (control == contract::kScroll) {
        m_scroll += delta;
        return {{QStringLiteral("scroll"), m_scroll}};
    }
    if (control == contract::kTrackFocus) {
        m_track = qBound(0, m_track + steps, 7);
        return {{QStringLiteral("track"), m_track}};
    }
    if (control == contract::kTrim) {
        m_trim += steps;
        return {{QStringLiteral("trim"), m_trim}, {QStringLiteral("tool"), m_context.value(QStringLiteral("tool"))}};
    }
    if (control == contract::kAudioGain) {
        m_gainDb = qBound(-60.0, m_gainDb + 0.1 * delta, 12.0);
        return {{QStringLiteral("gainDb"), m_gainDb}};
    }
    if (control == contract::kParamNudge) {
        const QString name = options.value(QStringLiteral("param"), valueAtFocus()).toString();
        if (!m_params.contains(name)) {
            return {{QStringLiteral("error"), QStringLiteral("no focused parameter")}};
        }
        const double step = options.value(QStringLiteral("step")).toString() == QLatin1String("fine") ? 0.1 : 1.0;
        const double v = qBound(0.0, m_params.value(name).toDouble() + delta * step, 100.0);
        m_params.insert(name, v);
        return {{QStringLiteral("param"), name}, {QStringLiteral("value"), v}};
    }
    if (control == QLatin1String("param.focus")) {
        const QString cur = valueAtFocus();
        const int idx = qBound(0, int(m_paramOrder.indexOf(cur)) + steps, int(m_paramOrder.size()) - 1);
        QVariantMap param = m_context.value(QStringLiteral("param")).toMap();
        param.insert(QStringLiteral("name"), m_paramOrder.at(idx));
        setContextValue(QStringLiteral("param"), param);
        return {{QStringLiteral("param"), m_paramOrder.at(idx)}};
    }
    if (control == contract::kColorWheel) {
        const QString wheel = options.value(QStringLiteral("wheel")).toString();
        const QString axis = options.value(QStringLiteral("axis"), QStringLiteral("luma")).toString();
        if (!m_wheels.contains(wheel)) {
            return {{QStringLiteral("error"), QStringLiteral("unknown wheel")}};
        }
        QVariantMap w = m_wheels.value(wheel).toMap();
        const double lo = wheel == QLatin1String("lift") ? -1.0 : 0.0;
        const double hi = wheel == QLatin1String("lift") ? 1.0 : 5.0;
        const double d = 0.01 * delta;
        const QStringList channels = axis == QLatin1String("luma") ? QStringList{QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")} : QStringList{axis};
        for (const auto &c : channels) {
            if (w.contains(c)) {
                w.insert(c, qBound(lo, w.value(c).toDouble() + d, hi));
            }
        }
        m_wheels.insert(wheel, w);
        return {{QStringLiteral("wheel"), wheel}, {QStringLiteral("axis"), axis}, {QStringLiteral("values"), w}};
    }
    if (control == contract::kCurveNudge) {
        const QString axis = options.value(QStringLiteral("axis"), QStringLiteral("value")).toString();
        m_curve.insert(axis, m_curve.value(axis).toDouble() + (axis == QLatin1String("time") ? steps : 0.01 * delta));
        return {{QStringLiteral("axis"), axis}, {QStringLiteral("offset"), m_curve.value(axis)}};
    }
    return {{QStringLiteral("error"), QStringLiteral("unknown control %1").arg(control)}};
}

QString MockKdenlive::valueAtFocus() const
{
    return m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString();
}

bool MockKdenlive::SetControlValue(const QString &control, double value, const QVariantMap &options)
{
    Q_UNUSED(options)
    record(QStringLiteral("SetControlValue %1 %2").arg(control).arg(value));
    if (control == contract::kShuttle) {
        m_shuttle = qBound(-7, int(std::lround(value)), 7);
        return true;
    }
    if (control == contract::kZoom) {
        m_zoom = qBound(0.0, value, 20.0);
        return true;
    }
    return false;
}

QVariantMap MockKdenlive::Invoke(const QString &command, const QVariantMap &args)
{
    record(QStringLiteral("Invoke %1 %2").arg(command, compact(args)));
    if (command == QLatin1String("colorwheel.reset")) {
        const QString wheel = args.value(QStringLiteral("wheel")).toString();
        if (!m_wheels.contains(wheel)) {
            return {{QStringLiteral("ok"), false}, {QStringLiteral("error"), QStringLiteral("unknown wheel")}};
        }
        m_wheels.insert(wheel, rgb(wheel == QLatin1String("lift") ? 0.0 : 1.0));
        return {{QStringLiteral("ok"), true}};
    }
    if (command == QLatin1String("param.reset")) {
        return {{QStringLiteral("ok"), true}};
    }
    return {{QStringLiteral("ok"), false}, {QStringLiteral("error"), QStringLiteral("unknown command")}};
}

void MockKdenlive::Notify(const QString &text, int timeoutMs)
{
    record(QStringLiteral("Notify \"%1\" %2ms").arg(text).arg(timeoutMs));
}

} // namespace cs
