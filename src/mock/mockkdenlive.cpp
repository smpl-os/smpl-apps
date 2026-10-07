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
    bool integral;
    bool editing;
    bool absolute;
    QStringList options;  // semantic options (part of the pending key)
};
// Limits and option sets follow Kdenlive's MR2 implementation: every relative
// delta is bounded by 10000, parameter deltas may be fractional, and all
// parameter controls/commands accept the same option set.
const QList<Descriptor> &descriptors()
{
    static const QList<Descriptor> d{
        {kJog, QStringLiteral("frames"), 1, 10000, true, false, false, {QStringLiteral("monitor"), QStringLiteral("scrub")}},
        {kShuttle, QStringLiteral("shuttle index steps"), 1, 10000, true, false, true, {QStringLiteral("monitor")}},
        {kZoom, QStringLiteral("zoom steps"), 1, 10000, true, false, false, {QStringLiteral("anchor")}},
        {kParamFocus, QStringLiteral("parameters"), 2, 64, true, false, false, {}},
        {kParamNudge, QStringLiteral("parameter display steps"), 2, 10000, false, true, false, {QStringLiteral("step"), QStringLiteral("keyframe"), QStringLiteral("axis")}},
        {kColorWheel, QStringLiteral("wheel steps"), 2, 10000, false, true, false, {QStringLiteral("wheel"), QStringLiteral("axis"), QStringLiteral("step"), QStringLiteral("keyframe")}},
        {kTrackFocus, QStringLiteral("tracks"), 3, 10000, true, false, false, {}},
        {kScroll, QStringLiteral("tenths of visible width"), 3, 10000, false, false, false, {}},
        {kAudioGain, QStringLiteral("0.1 dB"), 3, 10000, false, true, false, {}},
        {kTrim, QStringLiteral("frames"), 3, 10000, true, true, false, {QStringLiteral("edge"), QStringLiteral("mode")}},
    };
    return d;
}
// Accepted options per control/command beyond session/epoch, as in Kdenlive's
// qualified K23 (ControlSurface::optionsFor).
QStringList optionsFor(const QString &id)
{
    const QString t = QStringLiteral("target"), g = QStringLiteral("gesture"), ph = QStringLiteral("phase"), k = QStringLiteral("keyframe");
    if (id == kParamFocus || id == kTrackFocus || id == kScroll) {
        return {};
    }
    if (id == kCmdParamReset) {
        return {t, k};
    }
    if (id == kCmdWheelReset) {
        return {t, QStringLiteral("wheel"), k};
    }
    if (id == kParamNudge) {
        return {t, g, ph, QStringLiteral("step"), k};
    }
    if (id == kColorWheel) {
        return {t, g, ph, QStringLiteral("step"), k, QStringLiteral("wheel"), QStringLiteral("axis")};
    }
    if (id == kAudioGain) {
        return {t, g, ph};
    }
    if (id == kTrim) {
        return {t, g, ph, QStringLiteral("edge"), QStringLiteral("mode")};
    }
    if (id == kCmdTrackSet) {
        return {t, QStringLiteral("what"), QStringLiteral("value"), QStringLiteral("soloMode")};
    }
    if (id == kZoom) {
        return {QStringLiteral("anchor")};
    }
    if (id == kJog) {
        return {QStringLiteral("monitor"), QStringLiteral("scrub")};
    }
    return {QStringLiteral("monitor")};
}
constexpr int kMaxIdentifier = 128;
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
                              QStringLiteral("timeline")};
constexpr int kMaxGestureId = 128;
const QStringList kQualifiedTrimModes{QStringLiteral("resize")};
QVariantMap track(int id, const QString &type, const QString &label, bool targeted)
{
    return {{QStringLiteral("id"), id},           {QStringLiteral("type"), type},   {QStringLiteral("label"), label},
            {QStringLiteral("mute"), false},      {QStringLiteral("hide"), false},  {QStringLiteral("lock"), false},
            {QStringLiteral("solo"), false},      {QStringLiteral("target"), targeted}, {QStringLiteral("gainDb"), 0.0}};
}
QVariantMap clip(const QString &trk, int start, int end, int minStart, int maxEnd, const QString &linked, bool audio, double volumeDb, bool staticVolume)
{
    return {{QStringLiteral("track"), trk},        {QStringLiteral("start"), start},   {QStringLiteral("end"), end},
            {QStringLiteral("minStart"), minStart}, {QStringLiteral("maxEnd"), maxEnd}, {QStringLiteral("linked"), linked},
            {QStringLiteral("audio"), audio},       {QStringLiteral("volumeDb"), volumeDb}, {QStringLiteral("staticVolume"), staticVolume}};
}
double tenth(double v)
{
    return std::round(v * 10.0) / 10.0;
}
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
    m_trackOrder = {QStringLiteral("trk-4"), QStringLiteral("trk-3"), QStringLiteral("trk-7"), QStringLiteral("trk-8")};
    m_tracks = {{QStringLiteral("trk-4"), track(4, QStringLiteral("video"), QStringLiteral("V2"), false)},
                {QStringLiteral("trk-3"), track(3, QStringLiteral("video"), QStringLiteral("V1"), true)},
                {QStringLiteral("trk-7"), track(7, QStringLiteral("audio"), QStringLiteral("A1"), true)},
                {QStringLiteral("trk-8"), track(8, QStringLiteral("audio"), QStringLiteral("A2"), false)}};
    m_clips = {{QStringLiteral("clip-21"), clip(QStringLiteral("trk-3"), 100, 200, 50, 400, QStringLiteral("clip-22"), false, 0, false)},
               {QStringLiteral("clip-22"), clip(QStringLiteral("trk-7"), 100, 200, 50, 400, QStringLiteral("clip-21"), true, 0, true)},
               {QStringLiteral("clip-31"), clip(QStringLiteral("trk-8"), 300, 360, 300, 360, QString(), true, -3, false)}};
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
    m_context.insert(QStringLiteral("timeline"), timelineDescriptor());
}

QVariantMap MockKdenlive::timelineDescriptor() const
{
    // Before MR3 the timeline is described (zoom, native track identity) but
    // offers no handles. MR3 adds the handles of contract §MR3.
    const QString trackId = m_trackOrder.value(m_track);
    const QVariantMap t = m_tracks.value(trackId).toMap();
    QVariantMap trackDesc{{QStringLiteral("id"), t.value(QStringLiteral("id"))}, {QStringLiteral("sequence"), QStringLiteral("seq-1")}};
    QVariantMap timeline{{QStringLiteral("zoom"), m_zoom}, {QStringLiteral("track"), trackDesc}};
    if (m_stage < 3) {
        return timeline;
    }
    // Shapes as in Kdenlive's qualified MR3 (TimelineControl::context): native
    // integer ids, track state at the top level of timeline.track.
    const bool audio = t.value(QStringLiteral("type")).toString() == QLatin1String("audio");
    trackDesc.insert(kOptTarget, trackId);
    trackDesc.insert(QStringLiteral("audio"), audio);
    trackDesc.insert(QStringLiteral("name"), t.value(QStringLiteral("label")));
    trackDesc.insert(QStringLiteral("locked"), t.value(QStringLiteral("lock")));
    trackDesc.insert(QStringLiteral("muted"), audio && t.value(QStringLiteral("mute")).toBool());
    trackDesc.insert(QStringLiteral("hidden"), !audio && t.value(QStringLiteral("hide")).toBool());
    trackDesc.insert(QStringLiteral("targeted"), t.value(QStringLiteral("target")));
    trackDesc.insert(QStringLiteral("solo"), audio && t.value(QStringLiteral("solo")).toBool());
    if (audio) {
        // Only for an available audio mixer; the same handle as the track.
        trackDesc.insert(QStringLiteral("gain"), QVariantMap{{kOptTarget, trackId}, {QStringLiteral("value"), t.value(QStringLiteral("gainDb"))},
                                                             {QStringLiteral("min"), -60.0}, {QStringLiteral("max"), 12.0}, {QStringLiteral("enabled"), true}});
    }
    timeline.insert(QStringLiteral("track"), trackDesc);
    auto nativeId = [](const QString &key) { return key.section(QLatin1Char('-'), 1).toInt(); };
    QVariantList selection;
    if (!m_selectedClip.isEmpty()) {
        selection << nativeId(m_selectedClip);
    }
    timeline.insert(QStringLiteral("selection"), QVariantMap{{QStringLiteral("clips"), selection}, {QStringLiteral("count"), selection.size()}});
    if (!m_selectedClip.isEmpty()) {
        const QVariantMap c = m_clips.value(m_selectedClip).toMap();
        QStringList members{m_selectedClip};
        const QString linked = c.value(QStringLiteral("linked")).toString();
        if (!linked.isEmpty()) {  // aligned linked A/V pair
            members << linked;
        }
        std::sort(members.begin(), members.end());
        QVariantList clips, tracks;
        int gainTargets = 0;
        QString gainClip;
        for (const auto &m : members) {
            clips << nativeId(m);
            tracks << nativeId(m_clips.value(m).toMap().value(QStringLiteral("track")).toString());
            if (m_clips.value(m).toMap().value(QStringLiteral("audio")).toBool()) {  // an audio clip with one volume effect
                ++gainTargets;
                gainClip = m;
            }
        }
        timeline.insert(QStringLiteral("trim"), QVariantMap{{kOptTarget, QStringLiteral("trim-") + m_selectedClip}, {QStringLiteral("clip"), nativeId(m_selectedClip)},
                                                            {QStringLiteral("clips"), clips}, {QStringLiteral("tracks"), tracks},
                                                            {QStringLiteral("modes"), kQualifiedTrimModes}});
        if (gainTargets == 1) {
            timeline.insert(QStringLiteral("clipGain"), QVariantMap{{kOptTarget, QStringLiteral("gain-") + gainClip}, {QStringLiteral("clip"), nativeId(gainClip)},
                                                                    {QStringLiteral("unit"), QStringLiteral("dB")},
                                                                    {QStringLiteral("policy"), QStringLiteral("single_keyframe_existing_effect")}});
        }
    }
    return timeline;
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
                      {QStringLiteral("integral"), d.integral},
                      {QStringLiteral("maxDelta"), d.maxDelta},
                      {QStringLiteral("editing"), d.editing},
                      {QStringLiteral("absolute"), d.absolute},
                      {QStringLiteral("options"), d.options}};
        if (d.id == kColorWheel) {
            m.insert(QStringLiteral("axes"), QStringList{QStringLiteral("value"), QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")});
        }
        if (d.id == kTrim) {
            m.insert(QStringLiteral("modes"), kQualifiedTrimModes);
        }
        descs << m;
    }
    // Advertised (possible) keys, as Kdenlive does; optional ones appear only when they apply.
    QStringList contextKeys{QStringLiteral("serial"), QStringLiteral("emittedAtMs"), QStringLiteral("epoch"), QStringLiteral("ready"),
                            QStringLiteral("active"), QStringLiteral("dialog"), QStringLiteral("focus"), QStringLiteral("project"),
                            QStringLiteral("sequence"), QStringLiteral("activeMonitor"), QStringLiteral("position"), QStringLiteral("fps"),
                            QStringLiteral("playing"), QStringLiteral("speed"), QStringLiteral("tool"), QStringLiteral("timeline")};
    if (m_stage >= 2) {
        contextKeys << QStringLiteral("effect") << QStringLiteral("param") << QStringLiteral("colorWheel") << QStringLiteral("colorWheels")
                    << QStringLiteral("hoveredColorWheel");
    }
    QVariantMap limits{{QStringLiteral("subscriptions"), kMaxLeases}, {QStringLiteral("pendingKeys"), kMaxPendingKeys},
                       {QStringLiteral("queuedActions"), kMaxQueuedActions}, {QStringLiteral("contextHz"), 30},
                       {QStringLiteral("maximumDelta"), 10000}, {QStringLiteral("editingWriters"), 1}};
    if (m_stage >= 3) {
        limits.insert(QStringLiteral("trimGestureSteps"), m_trimGestureSteps);
    }
    return ok({{QStringLiteral("version"), uint(kVersion)},
               {QStringLiteral("revision"), uint(kRevision)},
               {QStringLiteral("implementation"), QStringLiteral("control-surface mock (stage %1)").arg(m_stage)},
               {QStringLiteral("controls"), controlsForStage()},
               {QStringLiteral("commands"), commandsForStage()},
               {QStringLiteral("contextKeys"), contextKeys},
               {QStringLiteral("limits"), limits},
               {QStringLiteral("trimModes"), m_stage >= 3 ? kQualifiedTrimModes : QStringList{}},
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
    if (it->watcher) {
        it->watcher->disconnect(this);
        it->watcher->deleteLater();  // may be called from the watcher's own signal
    }
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
    // Oversized counts, identifiers and strings: resource_limit; malformed or
    // out-of-range values: invalid_arguments.
    if (options.size() > kMaxOptions) {
        return fail(err::ResourceLimit, QStringLiteral("too many options"), QStringLiteral("options"));
    }
    *stringBudget -= stringSize(options);
    if (*stringBudget < 0) {
        return fail(err::ResourceLimit, QStringLiteral("input too large"), QStringLiteral("options"));
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
    return stateCheck();
}

QVariantMap MockKdenlive::stateCheck() const
{
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
    const QVariantMap timeline = m_context.value(QStringLiteral("timeline")).toMap();
    if (control == kTrim) {
        return timeline.value(QStringLiteral("trim")).toMap().value(kOptTarget).toString();
    }
    if (control == kCmdTrackSet) {
        return timeline.value(QStringLiteral("track")).toMap().value(kOptTarget).toString();
    }
    return {};
}

QVariantMap MockKdenlive::validateControl(const QString &control, double delta, const QVariantMap &options, QVariantMap *semantic) const
{
    const Descriptor *d = descriptor(control);
    if (!std::isfinite(delta)) {
        return fail(err::InvalidArguments, QStringLiteral("delta must be finite"), QStringLiteral("delta"));
    }
    if (d->integral && delta != std::trunc(delta)) {
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
    }
    if (control == kParamNudge && options.value(QStringLiteral("axis"), QStringLiteral("value")).toString() != QLatin1String("value")) {
        return fail(err::InvalidArguments, QStringLiteral("a numeric parameter has only a value axis"), QStringLiteral("axis"));
    }
    if (d->editing) {
        const QString gesture = options.value(kOptGesture).toString();
        if (gesture.isEmpty()) {
            return fail(err::InvalidArguments, QStringLiteral("an explicit gesture id is required"), kOptGesture);
        }
        if (gesture.size() > kMaxGestureId) {
            return fail(err::ResourceLimit, QStringLiteral("gesture id too long"), kOptGesture);
        }
        const QString target = options.value(kOptTarget).toString();
        if (control == kColorWheel) {
            // Each wheel of the focused widget has its own handle (colorWheels);
            // a hovered wheel is accepted only when the caller names its handle.
            const QVariantMap wheel = findWheelTarget(target);
            if (target.isEmpty() || wheel.isEmpty()) {
                return fail(err::TargetNotFound, QStringLiteral("unknown or stale wheel target"), kOptTarget);
            }
            if (options.contains(QStringLiteral("wheel")) && options.value(QStringLiteral("wheel")).toString() != wheel.value(QStringLiteral("wheel")).toString()) {
                return fail(err::UnsupportedParameter, QStringLiteral("the control does not match the bound wheel"), QStringLiteral("wheel"));
            }
            semantic->insert(QStringLiteral("wheel"), wheel.value(QStringLiteral("wheel")));
        } else if (control == kAudioGain) {
            if (target.isEmpty() || gainTrack(target).isEmpty()) {
                return fail(err::TargetNotFound, QStringLiteral("unknown or stale gain target"), kOptTarget);
            }
        } else if (target.isEmpty() || target != targetFor(control)) {
            return fail(err::TargetNotFound, QStringLiteral("unknown or stale target"), kOptTarget);
        }
        if (control == kTrim) {
            // mode is required; only the scope's advertised modes are qualified.
            const QStringList modes = m_context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("trim")).toMap().value(QStringLiteral("modes")).toStringList();
            if (!modes.contains(options.value(QStringLiteral("mode")).toString())) {
                return fail(err::UnsupportedMode, QStringLiteral("only the advertised trim modes are qualified"), QStringLiteral("mode"));
            }
            if (!QStringList{QStringLiteral("start"), QStringLiteral("end")}.contains(options.value(QStringLiteral("edge")).toString())) {
                return fail(err::InvalidArguments, QStringLiteral("an explicit edge (start/end) is required"), QStringLiteral("edge"));
            }
        }
        if (m_grouped && (control == kParamNudge || control == kColorWheel)) {
            return fail(err::UnsupportedGroup, QStringLiteral("grouped parameter propagation is not qualified"));
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
    if (!leaseFor(c)) {
        // Signals go only to current subscribers: a caller without a lease gets nothing.
        record(QStringLiteral("Control from %1 without a lease dropped").arg(c.owner));
        return;
    }
    const Descriptor *d = descriptor(control);
    const QString sentSession = options.value(kOptSession).toString();
    if (control.size() > kMaxIdentifier) {
        ack(c, sentSession, seq, control.left(kMaxIdentifier), fail(err::ResourceLimit, QStringLiteral("control identifier too long"), QStringLiteral("control")));
        return;
    }
    if (!d || d->stage > m_stage) {
        ack(c, sentSession, seq, control, fail(err::UnsupportedControl, QStringLiteral("control not offered"), QStringLiteral("control")));
        return;
    }
    const QStringList allowed = optionsFor(control);
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
    const QString session = l->session;
    const quint64 epoch = m_epoch;  // admitted against this epoch
    const QString phase = options.value(kOptPhase, QStringLiteral("update")).toString();
    if (phase == QLatin1String("end") || phase == QLatin1String("cancel")) {
        // As Kdenlive: a barrier first applies everything pending, then is
        // queued on its own (never merged into an earlier update).
        applyPending();
    }
    const QString gesture = options.value(kOptGesture).toString();
    const QString target = options.value(kOptTarget).toString();
    const QString key = QStringList{c.owner, session, QString::number(epoch), gesture, target, control, compact(semantic)}.join(QLatin1Char('|'));
    if (!m_pending.contains(key) && m_pending.size() >= kMaxPendingKeys) {
        ack(c, session, seq, control, fail(err::ResourceLimit, QStringLiteral("too many pending controls")));
        return;
    }
    if (m_pending.contains(key) && std::abs(m_pending.value(key).delta + delta) > d->maxDelta) {
        ack(c, session, seq, control, fail(err::ResourceLimit, QStringLiteral("accumulated delta too large")));
        return;
    }
    Pending &p = m_pending[key];
    if (p.firstSeq == 0) {
        p.caller = c;
        p.session = session;
        p.control = control;
        p.gesture = gesture;
        p.target = target;
        p.epoch = epoch;
        p.semantic = semantic;
        p.firstSeq = seq;
        p.phase = QStringLiteral("update");
        m_pendingOrder << key;
    }
    p.delta += delta;
    p.lastSeq = seq;
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

QVariant MockKdenlive::gestureValue(const Gesture &g) const
{
    if (g.control == kParamNudge) {
        if (g.frame >= 0) {
            QVariantMap keys;  // the whole key set: cancel also removes a created key
            const QMap<int, double> k = m_keys.value(g.param);
            for (auto it = k.cbegin(); it != k.cend(); ++it) {
                keys.insert(QString::number(it.key()), it.value());
            }
            return keys;
        }
        return m_params.value(g.param);
    }
    if (g.control == kColorWheel) {
        return m_wheels.value(g.param);
    }
    if (g.control == kAudioGain) {
        const QString id = g.param.section(QLatin1Char(':'), 1);
        return g.param.startsWith(QLatin1String("track:")) ? m_tracks.value(id).toMap().value(QStringLiteral("gainDb"))
                                                           : m_clips.value(id).toMap().value(QStringLiteral("volumeDb"));
    }
    if (g.control == kTrim) {
        QVariantMap bounds;
        for (const auto &id : g.param.split(QLatin1Char(','))) {
            const QVariantMap c = m_clips.value(id).toMap();
            bounds.insert(id, QVariantList{c.value(QStringLiteral("start")), c.value(QStringLiteral("end"))});
        }
        return bounds;
    }
    return {};
}
void MockKdenlive::restoreGestureValue(const Gesture &g, const QVariant &v)
{
    if (g.control == kParamNudge) {
        if (g.frame >= 0) {
            QMap<int, double> keys;
            const QVariantMap m = v.toMap();
            for (auto it = m.cbegin(); it != m.cend(); ++it) {
                keys.insert(it.key().toInt(), it.value().toDouble());
            }
            m_keys.insert(g.param, keys);
        } else {
            m_params.insert(g.param, v);
        }
    } else if (g.control == kColorWheel) {
        m_wheels.insert(g.param, v);
    } else if (g.control == kAudioGain) {
        const QString id = g.param.section(QLatin1Char(':'), 1);
        const bool isTrack = g.param.startsWith(QLatin1String("track:"));
        QVariantMap &store = isTrack ? m_tracks : m_clips;
        QVariantMap item = store.value(id).toMap();
        item.insert(isTrack ? QStringLiteral("gainDb") : QStringLiteral("volumeDb"), v);
        store.insert(id, item);
    } else if (g.control == kTrim) {
        const QVariantMap bounds = v.toMap();
        for (auto it = bounds.cbegin(); it != bounds.cend(); ++it) {
            QVariantMap c = m_clips.value(it.key()).toMap();
            c.insert(QStringLiteral("start"), it.value().toList().value(0));
            c.insert(QStringLiteral("end"), it.value().toList().value(1));
            m_clips.insert(it.key(), c);
        }
    }
}

QString MockKdenlive::gainTrack(const QString &target, QString *clipOut) const
{
    const QVariantMap timeline = m_context.value(QStringLiteral("timeline")).toMap();
    const QVariantMap trackGain = timeline.value(QStringLiteral("track")).toMap().value(QStringLiteral("gain")).toMap();
    const QVariantMap clipGain = timeline.value(QStringLiteral("clipGain")).toMap();
    if (!target.isEmpty() && trackGain.value(kOptTarget).toString() == target) {
        return target;  // the track handle doubles as its gain handle
    }
    if (!target.isEmpty() && clipGain.value(kOptTarget).toString() == target) {
        const QString clip = QStringLiteral("clip-%1").arg(clipGain.value(QStringLiteral("clip")).toInt());
        if (clipOut) {
            *clipOut = clip;
        }
        return m_clips.value(clip).toMap().value(QStringLiteral("track")).toString();
    }
    return {};
}

QString MockKdenlive::gestureSubject(const Pending &p) const
{
    if (p.control == kParamNudge) {
        return m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString();
    }
    if (p.control == kColorWheel) {
        return p.semantic.value(QStringLiteral("wheel")).toString();
    }
    if (p.control == kAudioGain) {
        QString clip;
        const QString track = gainTrack(p.target, &clip);
        return clip.isEmpty() ? QStringLiteral("track:") + track : QStringLiteral("clip:") + clip;
    }
    if (p.control == kTrim) {
        QStringList clips;
        for (const auto &id : m_context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("trim")).toMap().value(QStringLiteral("clips")).toList()) {
            clips << QStringLiteral("clip-%1").arg(id.toInt());
        }
        return clips.join(QLatin1Char(','));
    }
    return {};
}

QVariantMap MockKdenlive::preflightEdit(const Pending &p, int *frame) const
{
    auto refuse = [](const QString &code, const QString &message) { return QVariantMap{{QStringLiteral("code"), code}, {QStringLiteral("message"), message}}; };
    const bool playing = m_context.value(QStringLiteral("playing")).toBool();
    const bool create = p.semantic.value(QStringLiteral("keyframe")).toString() == QLatin1String("create");
    if (p.control == kParamNudge) {
        const QVariantMap param = m_context.value(QStringLiteral("param")).toMap();
        const QString name = param.value(QStringLiteral("name")).toString();
        if (param.value(QStringLiteral("managed")).toBool()) {
            return refuse(err::ManagedParameter, QStringLiteral("managed automation channel"));
        }
        if (!m_params.contains(name)) {
            return refuse(err::UnsupportedParameter, QStringLiteral("parameter type not supported"));
        }
        const bool multi = m_keys.contains(name);
        if (!multi && create) {
            return refuse(err::UnsupportedParameter, QStringLiteral("this parameter cannot create keyframes"));
        }
        if (playing && (multi || create)) {
            // Live grading covers static/single-key values only; writing a
            // different frame every tick would be automation recording.
            return refuse(err::Busy, QStringLiteral("multi-key edits and key creation need stopped playback"));
        }
        if (multi && !create && !m_keys.value(name).contains(m_position)) {
            return refuse(err::KeyframeRequired, QStringLiteral("no keyframe at the captured frame"));
        }
        *frame = multi ? m_position : -1;
    } else if (p.control == kColorWheel) {
        if (create) {  // the mock's wheels are static
            return refuse(err::UnsupportedParameter, QStringLiteral("this parameter cannot create keyframes"));
        }
    } else if (p.control == kAudioGain) {
        QString clip;
        const QString track = gainTrack(p.target, &clip);
        if (track.isEmpty()) {
            return refuse(err::TargetNotFound, QStringLiteral("unknown or stale gain target"));
        }
        const bool locked = m_tracks.value(track).toMap().value(QStringLiteral("lock")).toBool();
        if (clip.isEmpty() && locked) {
            return refuse(err::TrackLocked, QStringLiteral("an unlocked audio track in the active mixer is required"));
        }
        if (!clip.isEmpty() && locked) {
            return refuse(err::TargetNotFound, QStringLiteral("the unique unlocked clip-volume effect is no longer available"));
        }
        if (!clip.isEmpty() && !m_clips.value(clip).toMap().value(QStringLiteral("staticVolume")).toBool()) {
            return refuse(err::UnsupportedParameter, QStringLiteral("clip gain requires an unmanaged single-key volume effect"));
        }
    } else if (p.control == kTrim) {
        const QVariantList tracks = m_context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("trim")).toMap().value(QStringLiteral("tracks")).toList();
        for (const auto &t : tracks) {
            if (m_tracks.value(QStringLiteral("trk-%1").arg(t.toInt())).toMap().value(QStringLiteral("lock")).toBool()) {
                return refuse(err::TrackLocked, QStringLiteral("a track of the edit scope is locked"));
            }
        }
    }
    return {};
}

void MockKdenlive::finishFrameBoundGestures()
{
    const auto keys = m_gestures.keys();
    for (const auto &k : keys) {
        if (m_gestures.value(k).frame >= 0) {
            finishGesture(k, false, nullptr);
        }
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
        const QVariantMap refusal = stateCheck();  // dispatch check: state may have changed since admission
        if (!refusal.isEmpty()) {
            ack(p.caller, p.session, p.lastSeq, p.control, refusal);
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
            // One editing writer per host (limits.editingWriters = 1): another
            // caller's open gesture makes this busy and is left untouched.
            for (auto it = m_gestures.cbegin(); it != m_gestures.cend(); ++it) {
                if (it->owner != p.caller.owner) {
                    return error(err::Busy, QStringLiteral("another caller owns the current editing gesture"));
                }
            }
            // A live gesture id cannot change its target or control.
            const auto keys = m_gestures.keys();
            for (const auto &k : keys) {
                if (m_gestures.value(k).id == p.gesture) {
                    finishGesture(k, false, nullptr);
                    return error(err::HistoryConflict, QStringLiteral("a live gesture cannot change its target or control"));
                }
            }
            // A cancel needs a live gesture it owns (contract: also after end,
            // idle or history changes). Reusing an ended id with update/end starts
            // a new generation.
            if (p.phase == QLatin1String("cancel")) {
                return error(err::HistoryConflict, QStringLiteral("no live owned gesture to cancel"));
            }
        }
        int frame = -1;
        const QVariantMap refusal = preflightEdit(p, &frame);
        if (!refusal.isEmpty()) {
            return error(refusal.value(QStringLiteral("code")).toString(), refusal.value(QStringLiteral("message")).toString());
        }
        if (!m_gestures.contains(gk)) {
            finishAllGestures();  // the caller's own previous gesture ends first
            if (p.delta == 0) {
                return {};  // nothing to change: no gesture starts, nothing is owned
            }
            Gesture g;
            g.owner = p.caller.owner;
            g.id = p.gesture;
            g.control = p.control;
            g.target = p.target;
            g.semantic = p.semantic;
            g.param = gestureSubject(p);
            g.frame = frame;
            g.start = gestureValue(g);
            g.historyAtStart = int(m_history.size());
            m_gestures.insert(gk, g);
            m_gestureTimer->start();
        }
        m_gestures[gk].last.start();
    }
    if (!d->editing) {
        // As Kdenlive: applying any non-editing control (transport, zoom,
        // scroll, track or parameter focus) ends the editing gesture. Playback
        // clock ticks alone do not (see setPosition).
        finishAllGestures();
    }
    QVariantMap st;
    // Net-batch semantics: the summed delta is clamped once.
    if (p.control == kJog) {
        // Transport only, even in Slip mode: never touches clips or history.
        const int before = m_position;
        m_position = qBound(0, m_position + steps, m_duration);
        *changed = m_position != before;
        if (*changed) {
            finishFrameBoundGestures();  // seeking ends a multi-key edit
        }
        m_context.insert(QStringLiteral("position"), m_position);
        refreshDescriptors();
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
        if (*changed) {
            refreshDescriptors();
            bumpSerial(false);
        }
    } else if (p.control == kScroll) {
        m_scroll += p.delta;
        *changed = p.delta != 0;
        st = {{QStringLiteral("scroll"), m_scroll}};
    } else if (p.control == kTrackFocus) {
        const int before = m_track;
        m_track = qBound(0, m_track + steps, int(m_trackOrder.size()) - 1);
        *changed = m_track != before;
        st = {{QStringLiteral("track"), m_trackOrder.value(m_track)}};
        if (*changed) {
            refreshDescriptors();
            bumpSerial(true);  // a new track handle: new epoch, gestures end
        }
    } else if (p.control == kParamFocus) {
        const QString cur = m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString();
        const int idx = qBound(0, int(m_paramOrder.indexOf(cur)) + steps, int(m_paramOrder.size()) - 1);
        *changed = m_paramOrder.value(idx) != cur;
        if (*changed) {
            finishAllGestures();
            setContextValue(QStringLiteral("param"), paramDescriptor(m_paramOrder.at(idx)));
        }
        st = {{QStringLiteral("param"), m_paramOrder.value(idx)}};
    } else if (p.control == kParamNudge) {
        const Gesture &g = m_gestures[gk];
        const QString name = g.param;
        const double step = p.semantic.value(QStringLiteral("step")).toString() == QLatin1String("fine") ? 0.1 : 1.0;
        double before = m_params.value(name).toDouble();
        bool created = false;
        if (g.frame >= 0) {
            QMap<int, double> &keys = m_keys[name];
            if (!keys.contains(g.frame)) {  // explicit keyframe: create (checked in preflight)
                keys.insert(g.frame, before);
                created = true;
            }
            before = keys.value(g.frame);
        }
        const double v = qBound(0.0, tenth(before + p.delta * step), 100.0);
        if (g.frame >= 0) {
            m_keys[name].insert(g.frame, v);
        } else {
            m_params.insert(name, v);
        }
        *changed = created || v != before;
        st = {{QStringLiteral("param"), name}, {QStringLiteral("value"), v}, {QStringLiteral("frame"), g.frame}};
        refreshDescriptors();
        bumpSerial(false);
    } else if (p.control == kColorWheel) {
        const QString wheel = p.semantic.value(QStringLiteral("wheel")).toString();
        const QString axis = p.semantic.value(QStringLiteral("axis"), QStringLiteral("value")).toString();
        QVariantMap w = m_wheels.value(wheel).toMap();
        const double lo = wheel == QLatin1String("lift") ? -1.0 : 0.0;
        const double hi = wheel == QLatin1String("lift") ? 1.0 : wheel == QLatin1String("gamma") ? 2.0 : 4.0;
        const QStringList channels = axis == QLatin1String("value") ? QStringList{QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")} : QStringList{axis};
        const double step = p.semantic.value(QStringLiteral("step")).toString() == QLatin1String("fine") ? 0.001 : 0.01;
        for (const auto &ch : channels) {
            const double before = w.value(ch).toDouble();
            const double v = qBound(lo, std::round((before + step * p.delta) * 1e6) / 1e6, hi);
            *changed = *changed || v != before;
            w.insert(ch, v);
        }
        m_wheels.insert(wheel, w);
        st = {{QStringLiteral("wheel"), wheel}, {QStringLiteral("axis"), axis}, {QStringLiteral("values"), w}};
        refreshDescriptors();
        bumpSerial(false);
    } else if (p.control == kAudioGain) {
        const QString subject = m_gestures[gk].param;  // track:<id> or clip:<id>
        const bool isTrack = subject.startsWith(QLatin1String("track:"));
        QVariantMap &store = isTrack ? m_tracks : m_clips;
        const QString id = subject.section(QLatin1Char(':'), 1);
        const QString field = isTrack ? QStringLiteral("gainDb") : QStringLiteral("volumeDb");
        QVariantMap item = store.value(id).toMap();
        const double before = item.value(field).toDouble();
        // Track (mixer) gain rounds to 0.01 dB; clip volume keeps the raw sum.
        const double raw = before + 0.1 * p.delta;
        const double v = qBound(-60.0, isTrack ? std::round(raw * 100.0) / 100.0 : raw, 12.0);
        item.insert(field, v);
        store.insert(id, item);
        *changed = v != before;
        st = {{isTrack ? QStringLiteral("track") : QStringLiteral("clip"), id}, {QStringLiteral("gainDb"), v}};
        refreshDescriptors();
        bumpSerial(false);
    } else if (p.control == kTrim) {
        Gesture &g = m_gestures[gk];
        if (steps != 0 && g.steps >= m_trimGestureSteps) {
            return error(err::ResourceLimit, QStringLiteral("trim gesture step limit reached: end it before starting a new gesture"));
        }
        // Resize only: the declared clip(s) change; nothing downstream moves.
        const QStringList clips = g.param.split(QLatin1Char(','));
        const bool atEnd = p.semantic.value(QStringLiteral("edge")).toString() == QLatin1String("end");
        int applied = steps;
        for (const auto &id : clips) {
            const QVariantMap c = m_clips.value(id).toMap();
            const int start = c.value(QStringLiteral("start")).toInt(), end = c.value(QStringLiteral("end")).toInt();
            applied = atEnd ? qBound(start + 1 - end, applied, c.value(QStringLiteral("maxEnd")).toInt() - end)
                            : qBound(c.value(QStringLiteral("minStart")).toInt() - start, applied, end - 1 - start);
        }
        for (const auto &id : clips) {
            QVariantMap c = m_clips.value(id).toMap();
            const QString field = atEnd ? QStringLiteral("end") : QStringLiteral("start");
            c.insert(field, c.value(field).toInt() + applied);
            m_clips.insert(id, c);
        }
        *changed = applied != 0;
        if (*changed) {
            ++g.steps;
        }
        const QVariantMap first = m_clips.value(clips.value(0)).toMap();
        st = {{QStringLiteral("clip"), clips.value(0)},
              {QStringLiteral("start"), first.value(QStringLiteral("start"))},
              {QStringLiteral("end"), first.value(QStringLiteral("end"))},
              {QStringLiteral("mode"), p.semantic.value(QStringLiteral("mode"))}};
        refreshDescriptors();
        bumpSerial(false);
    }
    if (d->editing) {
        m_gestures[gk].changed = m_gestures[gk].changed || *changed;
        if (p.phase != QLatin1String("update")) {
            QVariantMap err;
            const bool owned = gestureValue(m_gestures[gk]) != m_gestures[gk].start;  // what a cancel would revert
            finishGesture(gk, p.phase == QLatin1String("cancel"), &err);
            if (!err.isEmpty()) {
                return error(err.value(QStringLiteral("code")).toString(), err.value(QStringLiteral("message")).toString());
            }
            if (p.phase == QLatin1String("cancel")) {
                // changed reports whether anything was reverted; values are the restored ones.
                *changed = owned;
                if (p.control == kColorWheel) {
                    st.insert(QStringLiteral("values"), m_wheels.value(p.semantic.value(QStringLiteral("wheel")).toString()));
                } else if (p.control == kParamNudge) {
                    const QString name = st.value(QStringLiteral("param")).toString();
                    const int frame = st.value(QStringLiteral("frame")).toInt();
                    st.insert(QStringLiteral("value"), frame >= 0 && m_keys.value(name).contains(frame) ? QVariant(m_keys.value(name).value(frame)) : m_params.value(name));
                } else if (p.control == kAudioGain) {
                    const QString track = st.value(QStringLiteral("track")).toString();
                    st.insert(QStringLiteral("gainDb"), track.isEmpty() ? m_clips.value(st.value(QStringLiteral("clip")).toString()).toMap().value(QStringLiteral("volumeDb"))
                                                                        : m_tracks.value(track).toMap().value(QStringLiteral("gainDb")));
                } else if (p.control == kTrim) {
                    const QVariantMap c = m_clips.value(st.value(QStringLiteral("clip")).toString()).toMap();
                    st.insert(QStringLiteral("start"), c.value(QStringLiteral("start")));
                    st.insert(QStringLiteral("end"), c.value(QStringLiteral("end")));
                }
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
    const bool netChanged = gestureValue(g) != g.start;
    if (cancel) {
        if (int(m_history.size()) == g.historyAtStart) {
            restoreGestureValue(g, g.start);
            refreshDescriptors();  // published values follow the restore (serial only)
            bumpSerial(false);
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
    if (id.size() > kMaxIdentifier) {
        return fail(err::ResourceLimit, QStringLiteral("action identifier too long"), QStringLiteral("id"));
    }
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
        const QVariantMap refusal = stateCheck();
        if (q.epoch != m_epoch) {
            outcome = fail(err::StaleContext, QStringLiteral("context changed before invocation"));
        } else if (!refusal.isEmpty()) {
            outcome = refusal;
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
    finishAllGestures();
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
    if (command.size() > kMaxIdentifier) {
        return fail(err::ResourceLimit, QStringLiteral("command identifier too long"), QStringLiteral("command"));
    }
    if (!commandsForStage().contains(command)) {
        return fail(err::UnsupportedControl, QStringLiteral("command not offered"), QStringLiteral("command"));
    }
    const QStringList allowed = optionsFor(command);
    int budget = kMaxStringInput - int(command.size());
    QVariantMap e = admit(c, args, allowed, &budget);
    if (!e.isEmpty()) {
        return e;
    }
    const QString target = args.value(kOptTarget).toString();
    for (auto it = m_gestures.cbegin(); it != m_gestures.cend(); ++it) {
        if (it->owner != c.owner) {
            return fail(err::Busy, QStringLiteral("another caller owns the editing gesture"));  // one editing writer
        }
    }
    if (!m_actions.isEmpty()) {
        return fail(err::Busy, QStringLiteral("a queued action must finish first"));
    }
    if (m_grouped && (command == kCmdWheelReset || command == kCmdParamReset)) {
        return fail(err::UnsupportedGroup, QStringLiteral("grouped parameter propagation is not qualified"));
    }
    finishAllGestures();  // context-changing discrete operations end gestures first
    if (command == kCmdWheelReset) {
        const QVariantMap descriptor = findWheelTarget(target);
        if (target.isEmpty() || descriptor.isEmpty()) {
            return fail(err::TargetNotFound, QStringLiteral("unknown or stale wheel target"), kOptTarget);
        }
        const QString wheel = descriptor.value(QStringLiteral("wheel")).toString();
        if (args.contains(QStringLiteral("wheel")) && args.value(QStringLiteral("wheel")).toString() != wheel) {
            return fail(err::UnsupportedParameter, QStringLiteral("the command does not match the bound wheel"), QStringLiteral("wheel"));
        }
        const QVariant def = rgb(wheel == QLatin1String("lift") ? 0.0 : 1.0);
        const bool changed = m_wheels.value(wheel) != def;
        m_wheels.insert(wheel, def);
        if (changed) {
            m_history << QStringLiteral("reset %1").arg(wheel);
            refreshDescriptors();
            bumpSerial(false);
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
            refreshDescriptors();
            bumpSerial(false);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    // track.set, validated in Kdenlive's order: a current track handle (any
    // track's issued handle), a boolean value, soloMode only with solo, then
    // what and its applicability.
    if (target.isEmpty() || !m_tracks.contains(target)) {
        return fail(err::TargetNotFound, QStringLiteral("an explicit current native track target is required"), kOptTarget);
    }
    if (args.value(QStringLiteral("value")).typeId() != QMetaType::Bool) {
        return fail(err::InvalidArguments, QStringLiteral("value must be boolean"), QStringLiteral("value"));
    }
    const QString what = args.value(QStringLiteral("what")).toString();
    if (what != QLatin1String("solo") && args.contains(QStringLiteral("soloMode"))) {
        return fail(err::InvalidArguments, QStringLiteral("soloMode applies only to solo"), QStringLiteral("soloMode"));
    }
    QVariantMap t = m_tracks.value(target).toMap();
    const bool audio = t.value(QStringLiteral("type")).toString() == QLatin1String("audio");
    const QString soloMode = args.value(QStringLiteral("soloMode"), QStringLiteral("exclusive")).toString();
    if (!QStringList{QStringLiteral("mute"), QStringLiteral("hide"), QStringLiteral("lock"), QStringLiteral("solo"), QStringLiteral("target")}.contains(what)) {
        return fail(err::UnsupportedMode, QStringLiteral("unsupported track property"), QStringLiteral("what"));
    }
    if ((what == QLatin1String("mute") || what == QLatin1String("hide")) && (what == QLatin1String("mute")) != audio) {
        return fail(err::UnsupportedMode, QStringLiteral("mute is audio-only and hide is video-only"), QStringLiteral("what"));
    }
    if (what == QLatin1String("solo") && (!audio || (soloMode != QLatin1String("exclusive") && soloMode != QLatin1String("additive")))) {
        return fail(err::UnsupportedMode, QStringLiteral("use an audio mixer track and an explicit solo policy"), QStringLiteral("soloMode"));
    }
    const bool value = args.value(QStringLiteral("value")).toBool();
    bool changed = t.value(what).toBool() != value;
    t.insert(what, value);
    m_tracks.insert(target, t);
    if (what == QLatin1String("solo") && value && soloMode == QLatin1String("exclusive")) {
        for (auto it = m_tracks.begin(); it != m_tracks.end(); ++it) {
            QVariantMap other = it.value().toMap();
            if (it.key() != target && other.value(QStringLiteral("solo")).toBool()) {
                other.insert(QStringLiteral("solo"), false);  // manual mute states are kept
                it.value() = other;
                changed = true;
            }
        }
    }
    if (changed) {
        m_history << QStringLiteral("track %1 %2=%3").arg(target, what, value ? QStringLiteral("on") : QStringLiteral("off"));
        refreshDescriptors();
        bumpSerial(false);
    }
    return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
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
        return fail(err::ResourceLimit, QStringLiteral("text too long"), QStringLiteral("text"));
    }
    if (text.isEmpty()) {
        return fail(err::InvalidArguments, QStringLiteral("text is empty"), QStringLiteral("text"));
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
    if (frame != m_position) {
        // Clock ticks and seeks leave whole-clip (live grading) gestures alone;
        // a multi-key edit's captured frame moved, so it ends.
        finishFrameBoundGestures();
    }
    m_position = frame;
    m_context.insert(QStringLiteral("position"), frame);
    refreshDescriptors();
    bumpSerial(false);
}

void MockKdenlive::setPlaying(bool on)
{
    m_context.insert(QStringLiteral("playing"), on);
    m_context.insert(QStringLiteral("speed"), on ? 1.0 : 0.0);
    bumpSerial(false);
}

void MockKdenlive::setParamMultiKey(const QString &name, bool on)
{
    if (!m_params.contains(name)) {
        return;
    }
    if (on) {
        const double v = m_params.value(name).toDouble();
        m_keys.insert(name, QMap<int, double>{{0, v}, {100, v}});
    } else {
        m_keys.remove(name);
    }
    refreshDescriptors();
    bumpSerial(true);  // the parameter's editing policy changed
}

void MockKdenlive::focusTrack(const QString &trackId)
{
    if (!m_trackOrder.contains(trackId)) {
        return;
    }
    m_track = int(m_trackOrder.indexOf(trackId));
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::selectClip(const QString &clipId)
{
    if (!clipId.isEmpty() && !m_clips.contains(clipId)) {
        return;
    }
    m_selectedClip = clipId;
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::addUnrelatedHistory(const QString &label)
{
    // As Kdenlive: an undo-stack change not made by the interface ends open
    // gestures and starts a new epoch.
    finishAllGestures();
    m_history << label;
    bumpSerial(true);
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
    const qint64 now = m_clock.elapsed();
    m_contextTimes << now;
    QVariantMap context = m_context;
    context.insert(QStringLiteral("emittedAtMs"), QVariant::fromValue<qulonglong>(quint64(now)));  // process-relative monotonic
    for (const auto &l : std::as_const(m_leases)) {
        ++m_contextSent[l.caller.owner];
        sendTo(l.caller, QStringLiteral("ContextChanged"), {QVariant::fromValue(context)});
    }
}

QVariantMap MockKdenlive::wheelDescriptor(const QString &wheel) const
{
    const QVariantMap native = m_wheels.value(wheel).toMap();
    QVariantMap values;
    for (const auto &c : {QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")}) {
        const double v = native.value(c).toDouble();
        values.insert(c, wheel == QLatin1String("lift") ? (v + 1.0) / 2.0 : v);  // displayed units
    }
    return {{QStringLiteral("target"), wheelTarget(wheel)},
            {QStringLiteral("wheel"), wheel},
            {QStringLiteral("name"), wheel + QStringLiteral("_r")},
            {QStringLiteral("axes"), QStringList{QStringLiteral("value"), QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")}},
            {QStringLiteral("values"), values},
            {QStringLiteral("min"), 0.0},
            {QStringLiteral("max"), wheel == QLatin1String("lift") ? 1.0 : wheel == QLatin1String("gamma") ? 2.0 : 4.0},
            {QStringLiteral("step"), 0.01},
            {QStringLiteral("fineStep"), 0.001},
            {QStringLiteral("frame"), -1},
            {QStringLiteral("keyframed"), false},
            {QStringLiteral("liveGrading"), true},
            {QStringLiteral("enabled"), true}};
}

QVariantMap MockKdenlive::paramDescriptor(const QString &name) const
{
    // A multi-key parameter is edited at the playhead's key (frame >= 0) and
    // only while stopped; static values are whole-clip (live grading).
    const bool multi = m_keys.contains(name);
    const QVariant value = multi ? (m_keys.value(name).contains(m_position) ? QVariant(m_keys.value(name).value(m_position)) : QVariant())
                                 : m_params.value(name);
    QVariantMap d{{QStringLiteral("target"), QStringLiteral("par-") + name},
                  {QStringLiteral("name"), name},
                  {QStringLiteral("type"), QStringLiteral("number")},
                  {QStringLiteral("unit"), QStringLiteral("%")},
                  {QStringLiteral("min"), 0.0},
                  {QStringLiteral("max"), 100.0},
                  {QStringLiteral("step"), 1.0},
                  {QStringLiteral("fineStep"), 0.1},
                  {QStringLiteral("frame"), multi ? m_position : -1},
                  {QStringLiteral("keyframed"), multi},
                  {QStringLiteral("liveGrading"), !multi},
                  {QStringLiteral("enabled"), true}};
    if (value.isValid()) {
        d.insert(QStringLiteral("value"), value);
    } else {
        d.insert(QStringLiteral("available"), false);  // no key at this frame
    }
    return d;
}

QVariantMap MockKdenlive::findWheelTarget(const QString &target) const
{
    if (target.isEmpty()) {
        return {};
    }
    for (const auto &entry : m_context.value(QStringLiteral("colorWheels")).toList()) {
        if (entry.toMap().value(kOptTarget).toString() == target) {
            return entry.toMap();
        }
    }
    for (const auto &key : {QStringLiteral("colorWheel"), QStringLiteral("hoveredColorWheel")}) {
        const QVariantMap d = m_context.value(key).toMap();
        if (d.value(kOptTarget).toString() == target) {
            return d;
        }
    }
    return {};
}

void MockKdenlive::refreshDescriptors()
{
    m_context.insert(QStringLiteral("timeline"), timelineDescriptor());
    // Values change without changing identities: serial only, never the epoch.
    if (m_context.contains(QStringLiteral("colorWheels"))) {
        QVariantList list;
        for (const auto &w : {QStringLiteral("lift"), QStringLiteral("gamma"), QStringLiteral("gain")}) {
            list << wheelDescriptor(w);
        }
        m_context.insert(QStringLiteral("colorWheels"), list);
    }
    for (const auto &key : {QStringLiteral("colorWheel"), QStringLiteral("hoveredColorWheel")}) {
        if (m_context.contains(key)) {
            m_context.insert(key, wheelDescriptor(m_context.value(key).toMap().value(QStringLiteral("wheel")).toString()));
        }
    }
    if (m_context.contains(QStringLiteral("param"))) {
        const QVariantMap p = m_context.value(QStringLiteral("param")).toMap();
        if (m_params.contains(p.value(QStringLiteral("name")).toString())) {
            QVariantMap d = paramDescriptor(p.value(QStringLiteral("name")).toString());
            for (const auto &k : {QStringLiteral("managed")}) {
                if (p.contains(k)) {
                    d.insert(k, p.value(k));
                }
            }
            m_context.insert(QStringLiteral("param"), d);
        }
    }
}

void MockKdenlive::focusWheels(const QString &focusedWheel)
{
    if (!focusedWheel.isEmpty() && !m_wheels.contains(focusedWheel)) {
        return;
    }
    if (focusedWheel.isEmpty()) {
        m_context.remove(QStringLiteral("colorWheel"));
        m_context.remove(QStringLiteral("colorWheels"));
        m_context.remove(QStringLiteral("effect"));
        m_context.insert(QStringLiteral("focus"), QStringLiteral("timeline"));
    } else {
        m_context.remove(QStringLiteral("param"));
        m_context.insert(QStringLiteral("focus"), QStringLiteral("effectStack"));
        m_context.insert(QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("lift_gamma_gain")}, {QStringLiteral("ownerId"), 12}, {QStringLiteral("sequence"), QStringLiteral("seq-1")}});
        m_context.insert(QStringLiteral("colorWheel"), wheelDescriptor(focusedWheel));
        m_context.insert(QStringLiteral("colorWheels"), QVariantList{});
        refreshDescriptors();
    }
    bumpSerial(true);
}

void MockKdenlive::hoverWheel(const QString &wheel)
{
    // Hover is not focus: it never changes the epoch.
    if (!wheel.isEmpty() && !m_wheels.contains(wheel)) {
        return;
    }
    if (wheel.isEmpty()) {
        m_context.remove(QStringLiteral("hoveredColorWheel"));
    } else {
        m_context.insert(QStringLiteral("hoveredColorWheel"), wheelDescriptor(wheel));
    }
    bumpSerial(false);
}

void MockKdenlive::focusParam(const QString &name)
{
    if (name.isEmpty()) {
        m_context.remove(QStringLiteral("param"));
        m_context.remove(QStringLiteral("effect"));
        m_context.insert(QStringLiteral("focus"), QStringLiteral("timeline"));
    } else {
        m_context.remove(QStringLiteral("colorWheel"));
        m_context.remove(QStringLiteral("colorWheels"));
        m_context.insert(QStringLiteral("focus"), QStringLiteral("effectStack"));
        m_context.insert(QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("brightness")}, {QStringLiteral("ownerId"), 12}, {QStringLiteral("sequence"), QStringLiteral("seq-1")}});
        m_context.insert(QStringLiteral("param"), paramDescriptor(name));
    }
    bumpSerial(true);
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
    QVariantMap keyState;
    for (auto it = m_keys.cbegin(); it != m_keys.cend(); ++it) {
        QVariantMap frames;
        for (auto k = it->cbegin(); k != it->cend(); ++k) {
            frames.insert(QString::number(k.key()), k.value());
        }
        keyState.insert(it.key(), frames);
    }
    return {{QStringLiteral("position"), m_position},
            {QStringLiteral("shuttle"), m_shuttle},
            {QStringLiteral("speed"), (m_shuttle < 0 ? -1 : 1) * kShuttleSpeeds[std::abs(m_shuttle)]},
            {QStringLiteral("zoom"), m_zoom},
            {QStringLiteral("scroll"), m_scroll},
            {QStringLiteral("track"), m_trackOrder.value(m_track)},
            {QStringLiteral("selectedClip"), m_selectedClip},
            {QStringLiteral("clips"), m_clips},
            {QStringLiteral("wheels"), m_wheels},
            {QStringLiteral("params"), m_params},
            {QStringLiteral("keys"), keyState},
            {QStringLiteral("tracks"), m_tracks},
            {QStringLiteral("triggered"), m_triggered},
            {QStringLiteral("history"), m_history},
            {QStringLiteral("epoch"), u64(m_epoch)}};
}

} // namespace cs
