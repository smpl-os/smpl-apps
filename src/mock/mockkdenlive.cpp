// SPDX-License-Identifier: GPL-2.0-or-later
#include "mockkdenlive.h"
#include "kdenlivecontract.h"

#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusServiceWatcher>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRandomGenerator>
#include <QTimer>
#include <algorithm>
#include <cmath>
#include <cstdio>

namespace cs {

namespace {
using namespace contract;

const double kShuttleSpeeds[] = {0.0, 1.0, 2.0, 4.0, 5.0, 8.0, 16.0, 60.0};  // Kdenlive jogaction.cpp

struct MockAction {
    const char *id;
    const char *text;
    const char *shortcut;
    bool checkable;
};
// Curated allowlist: quick, non-modal actions only.
const MockAction kActions[] = {
    {"monitor_play", "Play/Pause", "Space", false},
    {"monitor_pause", "Pause", "K", false},
    {"monitor_seek_snap_backward", "Go to Previous Snap Point", "Alt+Left", false},
    {"monitor_seek_snap_forward", "Go to Next Snap Point", "Alt+Right", false},
    {"mark_in", "Set Zone In", "I", false},
    {"mark_out", "Set Zone Out", "O", false},
    {"insert_to_in_point", "Insert Clip Zone in Timeline", "V", false},
    {"overwrite_to_in_point", "Overwrite Clip Zone in Timeline", "B", false},
    {"switch_monitor", "Switch Monitor", "T", false},
    {"cut_timeline_clip", "Cut Clip", "Shift+R", false},
    {"delete_timeline_clip", "Delete Selected Item", "Del", false},
    {"add_marker_guide_quickly", "Add Marker/Guide quickly", "Num+*", false},
    {"select_tool", "Selection Tool", "S", true},
    {"razor_tool", "Razor Tool", "X", true},
    {"ripple_tool", "Ripple Tool", "", true},
    {"roll_tool", "Roll Tool", "", true},
    {"slip_tool", "Slip Tool", "", true},
    {"slide_tool", "Slide Tool", "", true},
    {"keyframe_add", "Add/Remove Keyframe", "", false},
    {"keyframe_next", "Go to Next Keyframe", "", false},
    {"keyframe_previous", "Go to Previous Keyframe", "", false},
    {"zoom_fit", "Fit Zoom to Project", "", false},
    {"edit_undo", "Undo", "Ctrl+Z", false},
    {"edit_redo", "Redo", "Ctrl+Shift+Z", false},
};

struct Descriptor {
    QString id;
    QString unit;
    int stage;
    int maxDelta;
    bool editing;
    bool absolute;
    QStringList options;  // semantic options besides session/epoch (and target/gesture/phase for editing)
};
const QList<Descriptor> &descriptors()
{
    static const QList<Descriptor> d{
        {kJog, QStringLiteral("frames"), 1, 1000, false, false, {QStringLiteral("monitor"), QStringLiteral("scrub")}},
        {kShuttle, QStringLiteral("shuttle index steps"), 1, 14, false, true, {QStringLiteral("monitor")}},
        {kZoom, QStringLiteral("zoom steps"), 1, 20, false, false, {QStringLiteral("anchor")}},
        {kParamFocus, QStringLiteral("parameters"), 2, 10, false, false, {}},
        {kParamNudge, QStringLiteral("parameter display steps"), 2, 1000, true, false, {QStringLiteral("step"), QStringLiteral("keyframe")}},
        {kColorWheel, QStringLiteral("wheel steps (0.01)"), 2, 1000, true, false, {QStringLiteral("wheel"), QStringLiteral("axis")}},
        {kTrackFocus, QStringLiteral("tracks"), 3, 10, false, false, {}},
        {kScroll, QStringLiteral("tenths of visible width"), 3, 100, false, false, {}},
        {kAudioGain, QStringLiteral("0.1 dB"), 3, 600, true, false, {}},
        {kTrim, QStringLiteral("frames"), 3, 1000, true, false, {QStringLiteral("edge"), QStringLiteral("mode")}},
    };
    return d;
}
const Descriptor *descriptor(const QString &id)
{
    for (const auto &d : descriptors()) {
        if (d.id == id) {
            return &d;
        }
    }
    return nullptr;
}

QVariant u64(quint64 v)
{
    return QVariant::fromValue<qulonglong>(v);
}

QVariantMap rgb(double v)
{
    return {{QStringLiteral("r"), v}, {QStringLiteral("g"), v}, {QStringLiteral("b"), v}};
}

QString compact(const QVariantMap &m)
{
    return QString::fromUtf8(QJsonDocument(QJsonObject::fromVariantMap(m)).toJson(QJsonDocument::Compact));
}

int stringSize(const QVariant &v)
{
    if (v.typeId() == QMetaType::QString) {
        return int(v.toString().size());
    }
    if (v.typeId() == QMetaType::QVariantMap) {
        int n = 0;
        const QVariantMap m = v.toMap();
        for (auto it = m.begin(); it != m.end(); ++it) {
            n += int(it.key().size()) + stringSize(it.value());
        }
        return n;
    }
    if (v.typeId() == QMetaType::QVariantList || v.typeId() == QMetaType::QStringList) {
        int n = 0;
        for (const auto &e : v.toList()) {
            n += stringSize(e);
        }
        return n;
    }
    return 0;
}

const QStringList kTargetKeys{QStringLiteral("focus"), QStringLiteral("project"), QStringLiteral("sequence"), QStringLiteral("activeMonitor"),
                              QStringLiteral("tool"), QStringLiteral("effect"), QStringLiteral("param"), QStringLiteral("colorWheel"),
                              QStringLiteral("timeline"), QStringLiteral("audio"), QStringLiteral("edit")};
} // namespace

// ---------------------------------------------------------------------------
MockControlSurfaceAdaptor::MockControlSurfaceAdaptor(MockKdenlive *parent)
    : QDBusAbstractAdaptor(parent)
    , m(parent)
{
}
QVariantMap MockControlSurfaceAdaptor::Capabilities() { return m->capabilities(); }
QVariantMap MockControlSurfaceAdaptor::Subscribe() { return m->subscribe(); }
QVariantMap MockControlSurfaceAdaptor::Unsubscribe(const QString &session) { return m->unsubscribe(session); }
QVariantMap MockControlSurfaceAdaptor::GetContext() { return m->getContext(); }
QVariantMap MockControlSurfaceAdaptor::ListActions() { return m->listActions(); }
QVariantMap MockControlSurfaceAdaptor::TriggerAction(const QString &id, const QVariantMap &options) { return m->triggerAction(id, options); }
void MockControlSurfaceAdaptor::Control(const QString &control, double delta, const QVariantMap &options, qulonglong seq)
{
    m->control(control, delta, options, seq);
}
QVariantMap MockControlSurfaceAdaptor::SetControlValue(const QString &control, double value, const QVariantMap &options)
{
    return m->setControlValue(control, value, options);
}
QVariantMap MockControlSurfaceAdaptor::Invoke(const QString &command, const QVariantMap &args) { return m->invoke(command, args); }
QVariantMap MockControlSurfaceAdaptor::Notify(const QString &text, int timeoutMs, const QVariantMap &options) { return m->notify(text, timeoutMs, options); }

// ---------------------------------------------------------------------------
MockKdenlive::MockKdenlive(QObject *parent)
    : QObject(parent)
{
    qDBusRegisterMetaType<QList<QVariantMap>>();
    new MockControlSurfaceAdaptor(this);
    m_clock.start();
    m_contextTimer = new QTimer(this);
    m_contextTimer->setSingleShot(true);
    m_contextTimer->setTimerType(Qt::PreciseTimer);
    connect(m_contextTimer, &QTimer::timeout, this, [this] { emitContext(false); });
    m_gestureTimer = new QTimer(this);
    m_gestureTimer->setInterval(100);
    connect(m_gestureTimer, &QTimer::timeout, this, &MockKdenlive::checkGestureIdle);
    m_wheels = {{QStringLiteral("lift"), rgb(0.0)}, {QStringLiteral("gamma"), rgb(1.0)}, {QStringLiteral("gain"), rgb(1.0)}};
    m_paramOrder = {QStringLiteral("level"), QStringLiteral("opacity")};
    m_params = {{QStringLiteral("level"), 50.0}, {QStringLiteral("opacity"), 100.0}};
    m_tracks = {{QStringLiteral("trk-7"), QVariantMap{{QStringLiteral("type"), QStringLiteral("audio")}, {QStringLiteral("mute"), false}}},
                {QStringLiteral("trk-3"), QVariantMap{{QStringLiteral("type"), QStringLiteral("video")}, {QStringLiteral("hide"), false}}}};
    m_context = {{kCtxSerial, u64(0)},
                 {kCtxEpoch, u64(m_epoch)},
                 {QStringLiteral("ready"), true},
                 {QStringLiteral("active"), true},
                 {QStringLiteral("dialog"), false},
                 {QStringLiteral("focus"), QStringLiteral("timeline")},
                 {QStringLiteral("project"), QStringLiteral("mock-project-1")},
                 {QStringLiteral("sequence"), QStringLiteral("seq-1")},
                 {QStringLiteral("activeMonitor"), QStringLiteral("project")},
                 {QStringLiteral("position"), 0},
                 {QStringLiteral("fps"), QVariantMap{{QStringLiteral("num"), 25}, {QStringLiteral("den"), 1}}},
                 {QStringLiteral("playing"), false},
                 {QStringLiteral("speed"), 0.0},
                 {QStringLiteral("tool"), QStringLiteral("select")}};
    setStage(m_stage);
}

void MockKdenlive::setStage(int stage)
{
    m_stage = qBound(1, stage, 3);
    if (m_stage >= 3 && !m_context.contains(QStringLiteral("timeline"))) {
        // timeline.track: native id and sequence identity plus the opaque target.
        m_context.insert(QStringLiteral("timeline"),
                         QVariantMap{{QStringLiteral("track"), QVariantMap{{QStringLiteral("target"), QStringLiteral("trk-7")},
                                                                           {QStringLiteral("id"), 7},
                                                                           {QStringLiteral("sequence"), QStringLiteral("seq-1")},
                                                                           {QStringLiteral("type"), QStringLiteral("audio")},
                                                                           {QStringLiteral("label"), QStringLiteral("A1")}}}});
    } else if (m_stage < 3) {
        m_context.remove(QStringLiteral("timeline"));
    }
}

MockKdenlive::~MockKdenlive()
{
    for (auto &l : m_leases) {
        delete l.watcher;
    }
}

bool MockKdenlive::registerOn(QDBusConnection connection)
{
    return connection.registerObject(kPath, this, QDBusConnection::ExportAdaptors);
}

void MockKdenlive::record(const QString &line)
{
    log << line;
    if (m_print) {
        std::printf("%s\n", qPrintable(line));
        std::fflush(stdout);
    }
}

QVariantMap MockKdenlive::ok(const QVariantMap &result)
{
    return {{QStringLiteral("ok"), true}, {QStringLiteral("result"), result}};
}

QVariantMap MockKdenlive::fail(const QString &code, const QString &message, const QString &field)
{
    return {{QStringLiteral("ok"), false},
            {QStringLiteral("error"), QVariantMap{{QStringLiteral("code"), code}, {QStringLiteral("message"), message}, {QStringLiteral("field"), field}}}};
}

MockKdenlive::Caller MockKdenlive::currentCaller() const
{
    Caller c;
    if (!calledFromDBus()) {
        c.owner = QStringLiteral("local");
        return c;
    }
    const QDBusConnection conn = connection();
    c.connection = conn.name();
    const QString sender = message().service();
    if (sender.isEmpty()) {
        c.owner = QStringLiteral("peer:") + conn.name();
    } else {
        c.owner = sender;
        c.destination = sender;
    }
    return c;
}

MockKdenlive::Lease *MockKdenlive::leaseFor(const Caller &c)
{
    auto it = m_leases.find(c.owner);
    return it == m_leases.end() ? nullptr : &*it;
}

QStringList MockKdenlive::controlsForStage() const
{
    QStringList l;
    for (const auto &d : descriptors()) {
        if (d.stage <= m_stage) {
            l << d.id;
        }
    }
    return l;
}

QStringList MockKdenlive::commandsForStage() const
{
    QStringList l;
    if (m_stage >= 2) {
        l << kCmdParamReset << kCmdWheelReset;
    }
    if (m_stage >= 3) {
        l << kCmdTrackSet;
    }
    return l;
}

QVariantMap MockKdenlive::capabilities() const
{
    QList<QVariantMap> descs;
    for (const auto &d : descriptors()) {
        if (d.stage > m_stage) {
            continue;
        }
        QVariantMap m{{QStringLiteral("id"), d.id},
                      {QStringLiteral("unit"), d.unit},
                      {QStringLiteral("integral"), true},
                      {QStringLiteral("maxDelta"), d.maxDelta},
                      {QStringLiteral("editing"), d.editing},
                      {QStringLiteral("absolute"), d.absolute},
                      {QStringLiteral("options"), d.options}};
        if (d.id == kColorWheel) {
            m.insert(QStringLiteral("axes"), QStringList{QStringLiteral("value"), QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")});
        }
        if (d.id == kTrim) {
            m.insert(QStringLiteral("modes"), QStringList{QStringLiteral("ripple"), QStringLiteral("roll")});
        }
        descs << m;
    }
    return ok({{QStringLiteral("version"), uint(kVersion)},
               {QStringLiteral("revision"), uint(kRevision)},
               {QStringLiteral("implementation"), QStringLiteral("control-surface mock (stage %1)").arg(m_stage)},
               {QStringLiteral("controls"), controlsForStage()},
               {QStringLiteral("commands"), commandsForStage()},
               {QStringLiteral("contextKeys"), QStringList(m_context.keys())},
               {QStringLiteral("limits"), QVariantMap{{QStringLiteral("maxLeases"), kMaxLeases},
                                                      {QStringLiteral("maxPendingKeys"), kMaxPendingKeys},
                                                      {QStringLiteral("maxQueuedActions"), kMaxQueuedActions},
                                                      {QStringLiteral("maxOptions"), kMaxOptions},
                                                      {QStringLiteral("maxStringInput"), kMaxStringInput},
                                                      {QStringLiteral("maxNotifyText"), kMaxNotifyText},
                                                      {QStringLiteral("maxContextRateHz"), 30}}},
               {QStringLiteral("controlDescriptors"), QVariant::fromValue(descs)}});
}

QVariantMap MockKdenlive::subscribe()
{
    const Caller c = currentCaller();
    if (Lease *l = leaseFor(c)) {
        return ok({{QStringLiteral("session"), l->session}, {QStringLiteral("context"), m_context}});  // idempotent
    }
    if (m_leases.size() >= kMaxLeases) {
        return fail(err::ResourceLimit, QStringLiteral("too many subscribers"));
    }
    Lease l;
    l.caller = c;
    l.session = QStringLiteral("s%1-%2").arg(++m_sessionCounter).arg(QRandomGenerator::global()->generate(), 8, 16, QLatin1Char('0'));
    if (!c.destination.isEmpty()) {
        // Owner loss drops queued work and ends its applied gestures.
        l.watcher = new QDBusServiceWatcher(c.destination, QDBusConnection(c.connection), QDBusServiceWatcher::WatchForUnregistration, this);
        const QString owner = c.owner;
        connect(l.watcher, &QDBusServiceWatcher::serviceUnregistered, this, [this, owner] { dropLease(owner); });
    }
    m_leases.insert(c.owner, l);
    record(QStringLiteral("Subscribe %1 -> %2").arg(c.owner, l.session));
    return ok({{QStringLiteral("session"), l.session}, {QStringLiteral("context"), m_context}});
}

QVariantMap MockKdenlive::unsubscribe(const QString &session)
{
    const Caller c = currentCaller();
    Lease *l = leaseFor(c);
    if (!l || l->session != session) {
        return fail(err::NotSubscribed, QStringLiteral("no such session for this caller"), QStringLiteral("session"));
    }
    dropLease(c.owner);
    record(QStringLiteral("Unsubscribe %1").arg(c.owner));
    return ok({{QStringLiteral("unsubscribed"), true}});
}

void MockKdenlive::dropLease(const QString &owner)
{
    auto it = m_leases.find(owner);
    if (it == m_leases.end()) {
        return;
    }
    // Queued work of the owner is dropped silently (no one to tell); applied
    // gestures end as owned history entries.
    for (int i = int(m_pendingOrder.size()) - 1; i >= 0; --i) {
        const QString key = m_pendingOrder.at(i);
        if (m_pending.value(key).caller.owner == owner) {
            m_pending.remove(key);
            m_pendingOrder.removeAt(i);
        }
    }
    const auto gestureKeys = m_gestures.keys();
    for (const auto &gk : gestureKeys) {
        if (m_gestures.value(gk).owner == owner) {
            finishGesture(gk, false, nullptr);
        }
    }
    m_actions.erase(std::remove_if(m_actions.begin(), m_actions.end(), [&](const QueuedAction &a) { return a.caller.owner == owner; }), m_actions.end());
    delete it->watcher;
    m_leases.erase(it);
    if (m_leases.isEmpty()) {
        m_contextTimer->stop();  // no subscribers: no context work at all
    }
}

QVariantMap MockKdenlive::getContext() const
{
    return ok(m_context);
}

QVariantMap MockKdenlive::listActions() const
{
    QList<QVariantMap> list;
    for (const auto &a : kActions) {
        const QString id = QString::fromLatin1(a.id);
        list << QVariantMap{{QStringLiteral("id"), id},
                            {QStringLiteral("text"), QString::fromLatin1(a.text)},
                            {QStringLiteral("enabled"), true},
                            {QStringLiteral("checkable"), a.checkable},
                            {QStringLiteral("checked"), a.checkable && m_context.value(QStringLiteral("tool")).toString() + QStringLiteral("_tool") == id},
                            {QStringLiteral("shortcut"), QString::fromLatin1(a.shortcut)}};
    }
    return ok({{QStringLiteral("actions"), QVariant::fromValue(list)}});
}

QVariantMap MockKdenlive::admit(const Caller &c, const QVariantMap &options, const QStringList &allowed, int *stringBudget) const
{
    if (options.size() > kMaxOptions) {
        return fail(err::InvalidArguments, QStringLiteral("too many options"), QStringLiteral("options"));
    }
    *stringBudget -= stringSize(options);
    if (*stringBudget < 0) {
        return fail(err::InvalidArguments, QStringLiteral("input too large"), QStringLiteral("options"));
    }
    const auto it = m_leases.constFind(c.owner);
    if (it == m_leases.constEnd() || options.value(kOptSession).toString() != it->session) {
        return fail(err::NotSubscribed, QStringLiteral("a lease of this caller is required"), kOptSession);
    }
    for (auto o = options.begin(); o != options.end(); ++o) {
        if (o.key() != kOptSession && o.key() != kOptEpoch && !allowed.contains(o.key())) {
            return fail(err::InvalidArguments, QStringLiteral("unknown option"), o.key());
        }
    }
    if (!options.contains(kOptEpoch)) {
        return fail(err::InvalidArguments, QStringLiteral("epoch is required"), kOptEpoch);
    }
    if (options.value(kOptEpoch).toULongLong() != m_epoch) {
        return fail(err::StaleContext, QStringLiteral("context changed"), kOptEpoch);
    }
    if (!m_context.value(QStringLiteral("ready")).toBool()) {
        return fail(err::NotReady, QStringLiteral("editor not ready"));
    }
    if (m_context.value(QStringLiteral("closing")).toBool()) {
        return fail(err::Closing, QStringLiteral("editor closing"));
    }
    if (!m_context.value(QStringLiteral("active")).toBool()) {
        return fail(err::Inactive, QStringLiteral("application inactive"));
    }
    if (m_context.value(QStringLiteral("dialog")).toBool()) {
        return fail(err::Modal, QStringLiteral("modal dialog open"));
    }
    return {};
}

QString MockKdenlive::targetFor(const QString &control) const
{
    if (control == kParamNudge || control == kCmdParamReset) {
        return m_context.value(QStringLiteral("param")).toMap().value(kOptTarget).toString();
    }
    if (control == kColorWheel || control == kCmdWheelReset) {
        return m_context.value(QStringLiteral("colorWheel")).toMap().value(kOptTarget).toString();
    }
    if (control == kAudioGain) {
        return m_context.value(QStringLiteral("audio")).toMap().value(kOptTarget).toString();
    }
    if (control == kTrim) {
        return m_context.value(QStringLiteral("edit")).toMap().value(kOptTarget).toString();
    }
    return {};
}

QVariantMap MockKdenlive::validateControl(const QString &control, double delta, const QVariantMap &options, QVariantMap *semantic) const
{
    const Descriptor *d = descriptor(control);
    if (!std::isfinite(delta)) {
        return fail(err::InvalidArguments, QStringLiteral("delta must be finite"), QStringLiteral("delta"));
    }
    if (delta != std::trunc(delta)) {
        return fail(err::InvalidArguments, QStringLiteral("delta must be integral"), QStringLiteral("delta"));
    }
    if (std::abs(delta) > d->maxDelta) {
        return fail(err::InvalidArguments, QStringLiteral("delta out of range"), QStringLiteral("delta"));
    }
    auto oneOf = [&](const QString &key, const QStringList &values, QVariantMap *err) {
        if (options.contains(key) && !values.contains(options.value(key).toString())) {
            *err = fail(err::InvalidArguments, QStringLiteral("unsupported value"), key);
            return false;
        }
        return true;
    };
    QVariantMap e;
    if (!oneOf(QStringLiteral("monitor"), {QStringLiteral("active"), QStringLiteral("clip"), QStringLiteral("project")}, &e)
        || !oneOf(QStringLiteral("anchor"), {QStringLiteral("playhead"), QStringLiteral("mouse")}, &e)
        || !oneOf(QStringLiteral("step"), {QStringLiteral("normal"), QStringLiteral("fine")}, &e)
        || !oneOf(QStringLiteral("keyframe"), {QStringLiteral("existing"), QStringLiteral("create")}, &e)
        || !oneOf(QStringLiteral("wheel"), {QStringLiteral("lift"), QStringLiteral("gamma"), QStringLiteral("gain")}, &e)
        || !oneOf(QStringLiteral("edge"), {QStringLiteral("start"), QStringLiteral("end")}, &e)
        || !oneOf(kOptPhase, {QStringLiteral("update"), QStringLiteral("end"), QStringLiteral("cancel")}, &e)) {
        return e;
    }
    if (options.contains(QStringLiteral("scrub")) && options.value(QStringLiteral("scrub")).typeId() != QMetaType::Bool) {
        return fail(err::InvalidArguments, QStringLiteral("scrub must be boolean"), QStringLiteral("scrub"));
    }
    if (control == kColorWheel) {
        const QString axis = options.value(QStringLiteral("axis"), QStringLiteral("value")).toString();
        if (axis == QLatin1String("hue") || axis == QLatin1String("saturation")) {
            return fail(err::UnsupportedMode, QStringLiteral("axis not advertised"), QStringLiteral("axis"));
        }
        if (!QStringList{QStringLiteral("value"), QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")}.contains(axis)) {
            return fail(err::InvalidArguments, QStringLiteral("unknown axis"), QStringLiteral("axis"));
        }
        if (!options.contains(QStringLiteral("wheel"))) {
            return fail(err::InvalidArguments, QStringLiteral("wheel is required"), QStringLiteral("wheel"));
        }
    }
    if (control == kTrim) {
        const QString mode = options.value(QStringLiteral("mode")).toString();
        if (!options.contains(QStringLiteral("edge"))) {
            return fail(err::InvalidArguments, QStringLiteral("edge is required"), QStringLiteral("edge"));
        }
        if (mode != QLatin1String("ripple") && mode != QLatin1String("roll")) {
            return fail(err::UnsupportedMode, QStringLiteral("trim mode not qualified"), QStringLiteral("mode"));
        }
    }
    if (d->editing) {
        if (options.value(kOptGesture).toString().isEmpty()) {
            return fail(err::InvalidArguments, QStringLiteral("gesture is required"), kOptGesture);
        }
        const QString target = options.value(kOptTarget).toString();
        const QString hovered = m_context.value(QStringLiteral("hoveredColorWheel")).toMap().value(kOptTarget).toString();
        if (target.isEmpty() || (target != targetFor(control) && !(control == kColorWheel && !hovered.isEmpty() && target == hovered))) {
            return fail(err::TargetNotFound, QStringLiteral("unknown or stale target"), kOptTarget);
        }
    }
    for (auto o = options.begin(); o != options.end(); ++o) {
        if (d->options.contains(o.key())) {
            semantic->insert(o.key(), o.value());
        }
    }
    return {};
}

void MockKdenlive::sendTo(const Caller &c, const QString &member, const QVariantList &args)
{
    if (c.connection.isEmpty()) {
        return;  // local caller (tests calling directly)
    }
    QDBusMessage msg = c.destination.isEmpty() ? QDBusMessage::createSignal(kPath, kInterface, member)
                                               : QDBusMessage::createTargetedSignal(c.destination, kPath, kInterface, member);
    msg.setArguments(args);
    QDBusConnection(c.connection).send(msg);
}

void MockKdenlive::ack(const Caller &c, const QString &session, quint64 seq, const QString &control, const QVariantMap &outcome)
{
    // The outcome envelope carries the lease at top level, also on errors.
    QVariantMap o = outcome;
    o.insert(kOptSession, session);
    sendTo(c, QStringLiteral("ControlAck"), {u64(seq), control, QVariant::fromValue(o)});
}

void MockKdenlive::control(const QString &control, double delta, const QVariantMap &options, quint64 seq)
{
    ++m_controlMessages;
    const Caller c = currentCaller();
    const Descriptor *d = descriptor(control);
    const QString sentSession = options.value(kOptSession).toString();
    if (!d || d->stage > m_stage) {
        ack(c, sentSession, seq, control, fail(err::UnsupportedControl, QStringLiteral("control not offered"), QStringLiteral("control")));
        return;
    }
    QStringList allowed = d->options;
    if (d->editing) {
        allowed << kOptTarget << kOptGesture << kOptPhase;
    }
    int budget = kMaxStringInput - int(control.size());
    QVariantMap e = admit(c, options, allowed, &budget);
    QVariantMap semantic;
    if (e.isEmpty()) {
        e = validateControl(control, delta, options, &semantic);
    }
    Lease *l = leaseFor(c);
    if (e.isEmpty() && (!l || seq == 0 || seq <= l->lastSeq)) {
        e = fail(err::StaleSequence, QStringLiteral("sequence already seen"), QStringLiteral("seq"));
    }
    if (!e.isEmpty()) {
        ack(c, sentSession, seq, control, e);
        return;
    }
    l->lastSeq = seq;
    const QString gesture = options.value(kOptGesture).toString();
    const QString target = options.value(kOptTarget).toString();
    const QString key = QStringList{c.owner, l->session, QString::number(m_epoch), gesture, target, control, compact(semantic)}.join(QLatin1Char('|'));
    if (!m_pending.contains(key) && m_pending.size() >= kMaxPendingKeys) {
        ack(c, l->session, seq, control, fail(err::ResourceLimit, QStringLiteral("too many pending controls")));
        return;
    }
    Pending &p = m_pending[key];
    if (p.firstSeq == 0) {
        p.caller = c;
        p.session = l->session;
        p.control = control;
        p.gesture = gesture;
        p.target = target;
        p.epoch = m_epoch;
        p.semantic = semantic;
        p.firstSeq = seq;
        p.phase = QStringLiteral("update");
        m_pendingOrder << key;
    }
    p.delta += delta;
    p.lastSeq = seq;
    const QString phase = options.value(kOptPhase, QStringLiteral("update")).toString();
    if (phase == QLatin1String("cancel") || (phase == QLatin1String("end") && p.phase != QLatin1String("cancel"))) {
        p.phase = phase;
    }
    if (!m_applyScheduled) {
        // One apply per event-loop pass (or after the simulated GUI cost),
        // merging whatever arrives meanwhile.
        m_applyScheduled = true;
        QTimer::singleShot(m_applyDelayMs, this, &MockKdenlive::applyPending);
    }
}

QVariant MockKdenlive::gestureValue(const QString &control, const QVariantMap &semantic) const
{
    if (control == kParamNudge) {
        return m_params.value(m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString());
    }
    if (control == kColorWheel) {
        return m_wheels.value(semantic.value(QStringLiteral("wheel")).toString());
    }
    if (control == kAudioGain) {
        return m_gainDb;
    }
    if (control == kTrim) {
        return m_trim;
    }
    return {};
}

void MockKdenlive::restoreGestureValue(const QString &control, const QVariant &v)
{
    if (control == kParamNudge) {
        m_params.insert(m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString(), v);
    } else if (control == kAudioGain) {
        m_gainDb = v.toDouble();
    } else if (control == kTrim) {
        m_trim = v.toInt();
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
        const Lease *l = leaseFor(p.caller);
        if (!l || l->session != p.session) {
            continue;  // owner gone or unsubscribed: dropped silently
        }
        if (p.epoch != m_epoch) {
            ack(p.caller, p.session, p.lastSeq, p.control, fail(err::StaleContext, QStringLiteral("context changed before apply")));
            continue;
        }
        if (m_context.value(QStringLiteral("dialog")).toBool()) {
            ack(p.caller, p.session, p.lastSeq, p.control, fail(err::Modal, QStringLiteral("modal dialog open")));
            continue;
        }
        bool changed = false;
        QVariantMap st = applyOne(p, &changed);
        if (st.contains(QStringLiteral("error"))) {
            const QVariantMap err = st.value(QStringLiteral("error")).toMap();
            ack(p.caller, p.session, p.lastSeq, p.control, fail(err.value(QStringLiteral("code")).toString(), err.value(QStringLiteral("message")).toString()));
            continue;
        }
        st.insert(QStringLiteral("state"), QStringLiteral("applied"));
        st.insert(QStringLiteral("firstSeq"), u64(p.firstSeq));
        st.insert(QStringLiteral("lastSeq"), u64(p.lastSeq));
        st.insert(QStringLiteral("epoch"), u64(p.epoch));
        st.insert(QStringLiteral("gesture"), p.gesture);
        st.insert(QStringLiteral("target"), p.target);
        st.insert(QStringLiteral("changed"), changed);
        record(QStringLiteral("Control %1 %2 seq %3..%4 -> %5").arg(p.control).arg(p.delta).arg(p.firstSeq).arg(p.lastSeq).arg(compact(st)));
        ack(p.caller, p.session, p.lastSeq, p.control, ok(st));
    }
}

QVariantMap MockKdenlive::applyOne(const Pending &p, bool *changed)
{
    const int steps = int(p.delta);
    auto error = [](const QString &code, const QString &msg) {
        return QVariantMap{{QStringLiteral("error"), QVariantMap{{QStringLiteral("code"), code}, {QStringLiteral("message"), msg}}}};
    };
    const Descriptor *d = descriptor(p.control);
    QString gk;
    if (d->editing) {
        gk = QStringList{p.caller.owner, p.session, p.gesture, p.target, p.control, compact(p.semantic)}.join(QLatin1Char('|'));
        if (!m_gestures.contains(gk)) {
            Gesture g;
            g.owner = p.caller.owner;
            g.control = p.control;
            g.target = p.target;
            g.semantic = p.semantic;
            g.start = gestureValue(p.control, p.semantic);
            g.historyAtStart = int(m_history.size());
            m_gestures.insert(gk, g);
            m_gestureTimer->start();
        }
        m_gestures[gk].last.start();
    }
    QVariantMap st;
    // Net-batch semantics: the summed delta is clamped once.
    if (p.control == kJog) {
        // Transport only, even in Slip mode: never touches clips or history.
        const int before = m_position;
        m_position = qBound(0, m_position + steps, m_duration);
        *changed = m_position != before;
        m_context.insert(QStringLiteral("position"), m_position);
        bumpSerial(false);
        st = {{QStringLiteral("position"), m_position}};
    } else if (p.control == kShuttle) {
        const int before = m_shuttle;
        m_shuttle = qBound(-7, m_shuttle + steps, 7);
        *changed = m_shuttle != before;
        const double speed = (m_shuttle < 0 ? -1 : 1) * kShuttleSpeeds[std::abs(m_shuttle)];
        m_context.insert(QStringLiteral("playing"), m_shuttle != 0);  // zero is an explicit pause
        m_context.insert(QStringLiteral("speed"), speed);
        bumpSerial(false);
        st = {{QStringLiteral("shuttle"), m_shuttle}, {QStringLiteral("speed"), speed}};
    } else if (p.control == kZoom) {
        const int before = m_zoom;
        m_zoom = qBound(0, m_zoom + steps, 20);
        *changed = m_zoom != before;
        st = {{QStringLiteral("zoom"), m_zoom}};
    } else if (p.control == kScroll) {
        m_scroll += p.delta;
        *changed = p.delta != 0;
        st = {{QStringLiteral("scroll"), m_scroll}};
    } else if (p.control == kTrackFocus) {
        const int before = m_track;
        m_track = qBound(0, m_track + steps, 7);
        *changed = m_track != before;
        st = {{QStringLiteral("track"), m_track}};
    } else if (p.control == kParamFocus) {
        const QString cur = m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString();
        const int idx = qBound(0, int(m_paramOrder.indexOf(cur)) + steps, int(m_paramOrder.size()) - 1);
        *changed = m_paramOrder.value(idx) != cur;
        if (*changed) {
            QVariantMap param = m_context.value(QStringLiteral("param")).toMap();
            param.insert(QStringLiteral("name"), m_paramOrder.at(idx));
            param.insert(kOptTarget, QStringLiteral("par-%1").arg(m_paramOrder.at(idx)));
            setContextValue(QStringLiteral("param"), param);
        }
        st = {{QStringLiteral("param"), m_paramOrder.value(idx)}};
    } else if (p.control == kParamNudge) {
        const QVariantMap param = m_context.value(QStringLiteral("param")).toMap();
        if (param.value(QStringLiteral("managed")).toBool()) {
            return error(err::ManagedParameter, QStringLiteral("managed automation channel"));
        }
        if (param.value(QStringLiteral("animated")).toBool() && p.semantic.value(QStringLiteral("keyframe"), QStringLiteral("existing")) != QLatin1String("create")) {
            return error(err::KeyframeRequired, QStringLiteral("no keyframe at the captured frame"));
        }
        const QString name = param.value(QStringLiteral("name")).toString();
        if (!m_params.contains(name)) {
            return error(err::UnsupportedParameter, QStringLiteral("parameter type not supported"));
        }
        const double step = p.semantic.value(QStringLiteral("step")).toString() == QLatin1String("fine") ? 0.1 : 1.0;
        const double before = m_params.value(name).toDouble();
        const double v = qBound(0.0, before + p.delta * step, 100.0);
        m_params.insert(name, v);
        *changed = v != before;
        st = {{QStringLiteral("param"), name}, {QStringLiteral("value"), v}};
    } else if (p.control == kColorWheel) {
        const QString wheel = p.semantic.value(QStringLiteral("wheel")).toString();
        const QString axis = p.semantic.value(QStringLiteral("axis"), QStringLiteral("value")).toString();
        QVariantMap w = m_wheels.value(wheel).toMap();
        const double lo = wheel == QLatin1String("lift") ? -1.0 : 0.0;
        const double hi = wheel == QLatin1String("lift") ? 1.0 : wheel == QLatin1String("gamma") ? 2.0 : 4.0;
        const QStringList channels = axis == QLatin1String("value") ? QStringList{QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")} : QStringList{axis};
        for (const auto &ch : channels) {
            const double before = w.value(ch).toDouble();
            const double v = qBound(lo, before + 0.01 * p.delta, hi);
            *changed = *changed || v != before;
            w.insert(ch, v);
        }
        m_wheels.insert(wheel, w);
        st = {{QStringLiteral("wheel"), wheel}, {QStringLiteral("axis"), axis}, {QStringLiteral("values"), w}};
    } else if (p.control == kAudioGain) {
        const double before = m_gainDb;
        m_gainDb = qBound(-60.0, m_gainDb + 0.1 * p.delta, 12.0);
        *changed = m_gainDb != before;
        st = {{QStringLiteral("gainDb"), m_gainDb}};
    } else if (p.control == kTrim) {
        m_trim += steps;
        *changed = steps != 0;
        st = {{QStringLiteral("trim"), m_trim}, {QStringLiteral("mode"), p.semantic.value(QStringLiteral("mode"))}};
    }
    if (d->editing) {
        m_gestures[gk].changed = m_gestures[gk].changed || *changed;
        if (p.phase != QLatin1String("update")) {
            QVariantMap err;
            finishGesture(gk, p.phase == QLatin1String("cancel"), &err);
            if (!err.isEmpty()) {
                return error(err.value(QStringLiteral("code")).toString(), err.value(QStringLiteral("message")).toString());
            }
            st.insert(QStringLiteral("ended"), true);
        }
    }
    return st;
}

void MockKdenlive::finishGesture(const QString &key, bool cancel, QVariantMap *error)
{
    auto it = m_gestures.find(key);
    if (it == m_gestures.end()) {
        return;
    }
    const Gesture g = *it;
    m_gestures.erase(it);
    if (m_gestures.isEmpty()) {
        m_gestureTimer->stop();
    }
    const QVariantMap &semantic = g.semantic;
    const bool netChanged = gestureValue(g.control, semantic) != g.start;
    if (cancel) {
        if (int(m_history.size()) == g.historyAtStart) {
            if (g.control == kColorWheel) {
                m_wheels.insert(semantic.value(QStringLiteral("wheel")).toString(), g.start);
            } else {
                restoreGestureValue(g.control, g.start);
            }
            record(QStringLiteral("gesture %1 cancelled").arg(g.control));
            return;
        }
        if (error) {
            *error = {{QStringLiteral("code"), err::HistoryConflict}, {QStringLiteral("message"), QStringLiteral("gesture no longer owned")}};
        }
    }
    if (netChanged) {
        m_history << QStringLiteral("%1 gesture on %2").arg(g.control, g.target);  // one owned entry
    }
}

void MockKdenlive::finishAllGestures()
{
    const auto keys = m_gestures.keys();
    for (const auto &k : keys) {
        finishGesture(k, false, nullptr);
    }
}

void MockKdenlive::checkGestureIdle()
{
    const auto keys = m_gestures.keys();
    for (const auto &k : keys) {
        if (m_gestures.value(k).last.elapsed() > kGestureIdleMs) {
            finishGesture(k, false, nullptr);
        }
    }
}

void MockKdenlive::invalidatePending(const QString &reason)
{
    const QStringList order = m_pendingOrder;
    m_pendingOrder.clear();
    for (const auto &key : order) {
        const Pending p = m_pending.take(key);
        if (leaseFor(p.caller)) {
            ack(p.caller, p.session, p.lastSeq, p.control, fail(err::StaleContext, reason));
        }
    }
}

QVariantMap MockKdenlive::triggerAction(const QString &id, const QVariantMap &options)
{
    const Caller c = currentCaller();
    int budget = kMaxStringInput - int(id.size());
    QVariantMap e = admit(c, options, {}, &budget);
    if (!e.isEmpty()) {
        return e;
    }
    bool known = false;
    for (const auto &a : kActions) {
        known = known || id == QLatin1String(a.id);
    }
    if (!known) {
        return fail(err::UnknownAction, QStringLiteral("not offered to control surfaces"), QStringLiteral("id"));
    }
    if (m_actions.size() >= kMaxQueuedActions) {
        return fail(err::ResourceLimit, QStringLiteral("too many queued actions"));
    }
    QueuedAction q{c, leaseFor(c)->session, id, m_epoch, ++m_requestCounter};
    m_actions << q;
    QTimer::singleShot(m_applyDelayMs, this, &MockKdenlive::dispatchActions);
    record(QStringLiteral("TriggerAction %1 accepted as %2").arg(id).arg(q.requestId));
    return ok({{QStringLiteral("state"), QStringLiteral("accepted")}, {QStringLiteral("requestId"), u64(q.requestId)}, {QStringLiteral("id"), id}});
}

void MockKdenlive::dispatchActions()
{
    const QList<QueuedAction> queue = m_actions;
    m_actions.clear();
    for (const auto &q : queue) {
        const Lease *l = leaseFor(q.caller);
        if (!l || l->session != q.session) {
            continue;
        }
        QVariantMap outcome;
        if (q.epoch != m_epoch) {
            outcome = fail(err::StaleContext, QStringLiteral("context changed before invocation"));
        } else if (m_context.value(QStringLiteral("dialog")).toBool()) {
            outcome = fail(err::Modal, QStringLiteral("modal dialog open"));
        } else {
            finishAllGestures();  // discrete operations terminate editing gestures first
            m_triggered << q.id;
            if (q.id.endsWith(QLatin1String("_tool"))) {
                setContextValue(QStringLiteral("tool"), q.id.chopped(5));
            } else if (q.id == QLatin1String("monitor_play")) {
                m_context.insert(QStringLiteral("playing"), !m_context.value(QStringLiteral("playing")).toBool());
                bumpSerial(false);
            }
            outcome = ok({{QStringLiteral("state"), QStringLiteral("invoked")}, {QStringLiteral("id"), q.id}});
        }
        record(QStringLiteral("ActionFinished %1 %2").arg(q.requestId).arg(compact(outcome)));
        sendTo(q.caller, QStringLiteral("ActionFinished"), {u64(q.requestId), QVariant::fromValue(outcome)});
    }
}

QVariantMap MockKdenlive::setControlValue(const QString &control, double value, const QVariantMap &options)
{
    const Caller c = currentCaller();
    if (control != kShuttle || !controlsForStage().contains(control)) {
        return fail(err::UnsupportedControl, QStringLiteral("no absolute form"), QStringLiteral("control"));
    }
    int budget = kMaxStringInput;
    QVariantMap e = admit(c, options, {QStringLiteral("monitor")}, &budget);
    if (!e.isEmpty()) {
        return e;
    }
    if (!std::isfinite(value) || value != std::trunc(value) || value < -7 || value > 7) {
        return fail(err::InvalidArguments, QStringLiteral("shuttle index must be an integer in -7..7"), QStringLiteral("value"));
    }
    m_shuttle = int(value);
    const double speed = (m_shuttle < 0 ? -1 : 1) * kShuttleSpeeds[std::abs(m_shuttle)];
    m_context.insert(QStringLiteral("playing"), m_shuttle != 0);
    m_context.insert(QStringLiteral("speed"), speed);
    bumpSerial(false);
    return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("shuttle"), m_shuttle}, {QStringLiteral("speed"), speed}});
}

QVariantMap MockKdenlive::invoke(const QString &command, const QVariantMap &args)
{
    const Caller c = currentCaller();
    if (!commandsForStage().contains(command)) {
        return fail(err::UnsupportedControl, QStringLiteral("command not offered"), QStringLiteral("command"));
    }
    QStringList allowed{kOptTarget};
    if (command == kCmdWheelReset) {
        allowed << QStringLiteral("wheel");
    } else if (command == kCmdParamReset) {
        allowed << QStringLiteral("keyframe");
    } else if (command == kCmdTrackSet) {
        allowed << QStringLiteral("what") << QStringLiteral("value") << QStringLiteral("soloMode");
    }
    int budget = kMaxStringInput - int(command.size());
    QVariantMap e = admit(c, args, allowed, &budget);
    if (!e.isEmpty()) {
        return e;
    }
    const QString target = args.value(kOptTarget).toString();
    finishAllGestures();  // context-changing discrete operations end gestures first
    if (command == kCmdWheelReset) {
        const QString wheel = args.value(QStringLiteral("wheel")).toString();
        if (target.isEmpty() || target != targetFor(kCmdWheelReset)) {
            return fail(err::TargetNotFound, QStringLiteral("unknown or stale target"), kOptTarget);
        }
        if (!m_wheels.contains(wheel)) {
            return fail(err::InvalidArguments, QStringLiteral("unknown wheel"), QStringLiteral("wheel"));
        }
        const QVariant def = rgb(wheel == QLatin1String("lift") ? 0.0 : 1.0);
        const bool changed = m_wheels.value(wheel) != def;
        m_wheels.insert(wheel, def);
        if (changed) {
            m_history << QStringLiteral("reset %1").arg(wheel);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    if (command == kCmdParamReset) {
        if (target.isEmpty() || target != targetFor(kCmdParamReset)) {
            return fail(err::TargetNotFound, QStringLiteral("unknown or stale target"), kOptTarget);
        }
        const QString name = m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString();
        const bool changed = m_params.value(name).toDouble() != 50.0;
        m_params.insert(name, 50.0);
        if (changed) {
            m_history << QStringLiteral("reset %1").arg(name);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    // track.set
    if (!m_tracks.contains(target)) {
        return fail(err::TargetNotFound, QStringLiteral("unknown track"), kOptTarget);
    }
    QVariantMap t = m_tracks.value(target).toMap();
    const QString what = args.value(QStringLiteral("what")).toString();
    const bool audio = t.value(QStringLiteral("type")).toString() == QLatin1String("audio");
    if (!QStringList{QStringLiteral("mute"), QStringLiteral("hide"), QStringLiteral("lock"), QStringLiteral("solo"), QStringLiteral("target")}.contains(what)) {
        return fail(err::InvalidArguments, QStringLiteral("unknown property"), QStringLiteral("what"));
    }
    if ((what == QLatin1String("mute") || what == QLatin1String("solo")) != audio && what != QLatin1String("lock") && what != QLatin1String("target")) {
        return fail(err::UnsupportedMode, QStringLiteral("not applicable to this track type"), QStringLiteral("what"));
    }
    t.insert(what, args.value(QStringLiteral("value")).toBool());
    m_tracks.insert(target, t);
    m_history << QStringLiteral("track %1 %2").arg(target, what);
    return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), true}});
}

QVariantMap MockKdenlive::notify(const QString &text, int timeoutMs, const QVariantMap &options)
{
    const Caller c = currentCaller();
    int budget = kMaxStringInput - int(text.size());
    QVariantMap e = admit(c, options, {}, &budget);
    if (!e.isEmpty()) {
        return e;
    }
    if (text.size() > kMaxNotifyText) {
        return fail(err::InvalidArguments, QStringLiteral("text too long"), QStringLiteral("text"));
    }
    if (timeoutMs < 500 || timeoutMs > 5000) {
        return fail(err::InvalidArguments, QStringLiteral("timeout out of range"), QStringLiteral("timeoutMs"));
    }
    record(QStringLiteral("Notify \"%1\" %2ms").arg(text).arg(timeoutMs));
    return ok({{QStringLiteral("shown"), true}});
}

// ---------------------------------------------------------------------------
void MockKdenlive::setContextValue(const QString &key, const QVariant &value)
{
    if (value.isValid()) {
        m_context.insert(key, value);
    } else {
        m_context.remove(key);
    }
    bumpSerial(kTargetKeys.contains(key));
}

void MockKdenlive::setPosition(int frame)
{
    m_position = frame;
    m_context.insert(QStringLiteral("position"), frame);
    bumpSerial(false);
}

void MockKdenlive::addUnrelatedHistory(const QString &label)
{
    m_history << label;
}

void MockKdenlive::bumpSerial(bool epoch)
{
    m_context.insert(kCtxSerial, u64(++m_serial));
    if (epoch) {
        m_context.insert(kCtxEpoch, u64(++m_epoch));
        // Focus/target changes invalidate queued work and end gestures.
        invalidatePending(QStringLiteral("context changed"));
        finishAllGestures();
    }
    emitContext(false);
}

void MockKdenlive::emitContext(bool force)
{
    Q_UNUSED(force)
    if (m_leases.isEmpty()) {
        return;  // no subscribers: no timers, no signals
    }
    if (m_lastContextEmit.isValid() && m_lastContextEmit.elapsed() < kMinContextIntervalMs) {
        if (!m_contextTimer->isActive()) {
            m_contextTimer->start(int(kMinContextIntervalMs - m_lastContextEmit.elapsed()));
        }
        return;
    }
    m_lastContextEmit.start();
    m_contextTimes << m_clock.elapsed();
    for (const auto &l : std::as_const(m_leases)) {
        ++m_contextSent[l.caller.owner];
        sendTo(l.caller, QStringLiteral("ContextChanged"), {QVariant::fromValue(m_context)});
    }
}

int MockKdenlive::contextSignalsTotal() const
{
    int n = 0;
    for (auto v : m_contextSent) {
        n += v;
    }
    return n;
}

QVariantMap MockKdenlive::state() const
{
    return {{QStringLiteral("position"), m_position},
            {QStringLiteral("shuttle"), m_shuttle},
            {QStringLiteral("speed"), (m_shuttle < 0 ? -1 : 1) * kShuttleSpeeds[std::abs(m_shuttle)]},
            {QStringLiteral("zoom"), m_zoom},
            {QStringLiteral("scroll"), m_scroll},
            {QStringLiteral("track"), m_track},
            {QStringLiteral("trim"), m_trim},
            {QStringLiteral("gainDb"), m_gainDb},
            {QStringLiteral("wheels"), m_wheels},
            {QStringLiteral("params"), m_params},
            {QStringLiteral("tracks"), m_tracks},
            {QStringLiteral("triggered"), m_triggered},
            {QStringLiteral("history"), m_history},
            {QStringLiteral("epoch"), u64(m_epoch)}};
}

} // namespace cs
