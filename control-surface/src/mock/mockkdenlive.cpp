// SPDX-License-Identifier: GPL-2.0-or-later
#include "mockkdenlive.h"
#include "kdenlivecatalog.h"
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

// The curated actions and the editing/playback sets live in the shared catalog
// (src/daemon/kdenlivecatalog.cpp), which list-actions also prints.
const QStringList kEditingActions = catalog::editingActionIds();
const QStringList kPlaybackActions = catalog::playbackActionIds();
// Playback-starting actions besides the play toggle (their effect in the mock).
const QStringList kPlaybackStart{QStringLiteral("monitor_seek_backward"), QStringLiteral("monitor_seek_forward"), QStringLiteral("monitor_play_zone"),
                                 QStringLiteral("monitor_play_zone_cursor"), QStringLiteral("monitor_loop_zone"), QStringLiteral("monitor_loop_clip")};
bool isEditingAction(const QString &id)
{
    return catalog::isEditingAction(id);  // the fixed ids plus the camera, tag and effect families (MR1b-A)
}
// effect_<id> exists for every installed effect; the mock offers these common
// ones (all present in a standard Kdenlive install; avfilter.acompressor is
// on Kdenlive's excluded_effects.txt, so the compressor is avfilter.compand).
const QStringList kMockEffects{
    QStringLiteral("lift_gamma_gain"), QStringLiteral("volume"), QStringLiteral("fadein"), QStringLiteral("fadeout"), QStringLiteral("fade_from_black"),
    QStringLiteral("fade_to_black"), QStringLiteral("qtcrop"), QStringLiteral("qtblend"), QStringLiteral("avfilter.gblur"), QStringLiteral("frei0r.vignette"),
    QStringLiteral("frei0r.sharpness"), QStringLiteral("avfilter.curves"), QStringLiteral("avfilter.colorlevels"), QStringLiteral("avfilter.huesaturation"),
    QStringLiteral("avfilter.exposure"), QStringLiteral("avfilter.colortemperature"), QStringLiteral("avfilter.lut3d"), QStringLiteral("frei0r.sopsat"),
    QStringLiteral("chroma"), QStringLiteral("avfilter.hsvkey"), QStringLiteral("lumakey"), QStringLiteral("avfilter.despill"), QStringLiteral("mask_start-shape"),
    QStringLiteral("mask_start-rotoscoping"), QStringLiteral("mask_apply"), QStringLiteral("freeze"), QStringLiteral("dynamictext"), QStringLiteral("timer"),
    QStringLiteral("obscure"), QStringLiteral("frei0r.pixeliz0r"), QStringLiteral("audiopan"), QStringLiteral("avfilter.equalizer"),
    QStringLiteral("avfilter.compand"), QStringLiteral("dynamic_loudness"), QStringLiteral("avfilter.highpass"), QStringLiteral("dropshadow"),
    QStringLiteral("frei0r.glow"), QStringLiteral("avfilter.hflip"), QStringLiteral("frei0r.select0r")};
constexpr int kMockLayoutSlots = 5;  // Kdenlive's default layouts: Logging, Editing, Audio, Effects, Color
constexpr int kMockProjectTags = 5;  // a new project's tags: Red, Green, Blue, Yellow, Cyan

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
        {kTimelineTarget, QStringLiteral("tracks"), 4, 10000, true, false, false, {QStringLiteral("kind")}},
        {kPan, QStringLiteral("balance units"), 4, 10000, true, true, false, {}},
        {kNudge, QStringLiteral("frames"), 4, 10000, true, true, false, {QStringLiteral("unit")}},
        {kEffectFocus, QStringLiteral("effects"), 4, 10000, true, false, false, {}},
        {kBinCursor, QStringLiteral("bin rows"), 4, 10000, true, false, false, {QStringLiteral("extend")}},
        {kBinRating, QStringLiteral("stars"), 4, 10000, true, true, false, {}},
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
    if (id == kNudge) {
        return {t, g, ph, QStringLiteral("unit")};
    }
    if (id == kTimelineTarget) {
        return {QStringLiteral("kind")};
    }
    if (id == kPan || id == kBinRating) {
        return {t, g, ph};
    }
    if (id == kEffectFocus) {
        return {};
    }
    if (id == kBinCursor) {
        return {QStringLiteral("extend")};
    }
    if (id == kCmdEffectAdd) {
        return {t, QStringLiteral("id"), QStringLiteral("preset")};
    }
    if (id == kCmdEffectSet || id == kCmdStackSet) {
        return {t, QStringLiteral("what"), QStringLiteral("value")};
    }
    if (id == kCmdEffectMove) {
        return {t, QStringLiteral("delta")};
    }
    if (id == kCmdEffectRemove) {
        return {t};
    }
    if (id == kCmdBinTag) {
        return {t, QStringLiteral("tag"), QStringLiteral("value")};
    }
    if (id == kCmdBinSelect) {
        return {QStringLiteral("tag")};
    }
    if (id == kCmdBinFilter) {
        return {QStringLiteral("tag"), QStringLiteral("rating"), QStringLiteral("clear")};
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
                              QStringLiteral("timeline"), QStringLiteral("bin")};
constexpr int kMaxGestureId = 128;
const QStringList kQualifiedTrimModes{QStringLiteral("resize")};
const QStringList kStage4TrimModes{QStringLiteral("resize"), QStringLiteral("slip"), QStringLiteral("ripple")};
QVariantMap track(int id, const QString &type, const QString &label, bool targeted)
{
    return {{QStringLiteral("id"), id},           {QStringLiteral("type"), type},   {QStringLiteral("label"), label},
            {QStringLiteral("mute"), false},      {QStringLiteral("hide"), false},  {QStringLiteral("lock"), false},
            {QStringLiteral("solo"), false},      {QStringLiteral("target"), targeted}, {QStringLiteral("gainDb"), 0.0},
            {QStringLiteral("pan"), 0}};
}
QVariantMap clip(const QString &trk, int start, int end, int minStart, int maxEnd, const QString &linked, bool audio, double volumeDb, bool staticVolume)
{
    const int srcIn = start - minStart;
    return {{QStringLiteral("track"), trk},        {QStringLiteral("start"), start},   {QStringLiteral("end"), end},
            {QStringLiteral("minStart"), minStart}, {QStringLiteral("maxEnd"), maxEnd}, {QStringLiteral("linked"), linked},
            {QStringLiteral("audio"), audio},       {QStringLiteral("volumeDb"), volumeDb}, {QStringLiteral("staticVolume"), staticVolume},
            {QStringLiteral("srcIn"), srcIn},        {QStringLiteral("srcOut"), srcIn + (end - start)}, {QStringLiteral("sourceLength"), maxEnd - minStart},
            {QStringLiteral("mixed"), false}};
}
QVariantMap binClip(const QString &id, const QString &name, const QStringList &tags, int rating)
{
    return {{QStringLiteral("id"), id}, {QStringLiteral("name"), name}, {QStringLiteral("tags"), tags}, {QStringLiteral("rating"), rating * 2}, {QStringLiteral("complete"), true}};
}
QVariantMap effect(const QString &id, bool enabled = true, bool builtin = false, bool group = false)
{
    return {{QStringLiteral("id"), id}, {QStringLiteral("enabled"), enabled}, {QStringLiteral("builtin"), builtin}, {QStringLiteral("group"), group}};
}
int nativeId(const QString &key)
{
    return key.section(QLatin1Char('-'), 1).toInt();
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
    connect(m_contextTimer, &QTimer::timeout, this, [this] { emitContext(true); });
    m_gestureTimer = new QTimer(this);
    m_gestureTimer->setInterval(100);
    connect(m_gestureTimer, &QTimer::timeout, this, &MockKdenlive::checkGestureIdle);
    m_wheels = {{QStringLiteral("lift"), rgb(0.0)}, {QStringLiteral("gamma"), rgb(1.0)}, {QStringLiteral("gain"), rgb(1.0)}};
    m_paramOrder = {QStringLiteral("level"), QStringLiteral("opacity")};
    m_params = {{QStringLiteral("level"), 50.0},       {QStringLiteral("opacity"), 100.0}, {QStringLiteral("rOffset"), 0.0},
                {QStringLiteral("gOffset"), 0.0},     {QStringLiteral("bOffset"), 0.0},   {QStringLiteral("saturation"), 1.0}};
    m_trackOrder = {QStringLiteral("trk-4"), QStringLiteral("trk-3"), QStringLiteral("trk-7"), QStringLiteral("trk-8")};
    m_tracks = {{QStringLiteral("trk-4"), track(4, QStringLiteral("video"), QStringLiteral("V2"), false)},
                {QStringLiteral("trk-3"), track(3, QStringLiteral("video"), QStringLiteral("V1"), true)},
                {QStringLiteral("trk-7"), track(7, QStringLiteral("audio"), QStringLiteral("A1"), true)},
                {QStringLiteral("trk-8"), track(8, QStringLiteral("audio"), QStringLiteral("A2"), false)}};
    m_clips = {{QStringLiteral("clip-21"), clip(QStringLiteral("trk-3"), 100, 200, 50, 400, QStringLiteral("clip-22"), false, 0, false)},
               {QStringLiteral("clip-22"), clip(QStringLiteral("trk-7"), 100, 200, 50, 400, QStringLiteral("clip-21"), true, 0, true)},
               {QStringLiteral("clip-31"), clip(QStringLiteral("trk-8"), 300, 360, 300, 360, QString(), true, -3, false)},
               {QStringLiteral("clip-41"), clip(QStringLiteral("trk-3"), 250, 310, 200, 520, QStringLiteral("clip-42"), false, 0, false)},
               {QStringLiteral("clip-42"), clip(QStringLiteral("trk-7"), 250, 310, 200, 520, QStringLiteral("clip-41"), true, 0, true)}};
    m_videoTarget = QStringLiteral("trk-3");
    m_audioTargets = {{QStringLiteral("0"), QStringLiteral("trk-7")}};
    m_binClips = {binClip(QStringLiteral("bin-1"), QStringLiteral("A001"), {QStringLiteral("#ff0000")}, 3),
                  binClip(QStringLiteral("bin-2"), QStringLiteral("B002"), {QStringLiteral("#0000ff"), QStringLiteral("#ff0000")}, 3),
                  binClip(QStringLiteral("bin-3"), QStringLiteral("C003"), {}, 0)};
    m_effectStack = {effect(QStringLiteral("lift_gamma_gain"))};
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
    m_actionsSent = actionList();
}

void MockKdenlive::setStage(int stage)
{
    m_stage = qBound(1, stage, 4);
    refreshDescriptors();
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
    const bool routed = m_stage >= 4 ? (audio ? m_audioTargets.values().contains(trackId) : m_videoTarget == trackId) : t.value(QStringLiteral("target")).toBool();
    trackDesc.insert(QStringLiteral("targeted"), routed);
    trackDesc.insert(QStringLiteral("solo"), audio && t.value(QStringLiteral("solo")).toBool());
    if (audio) {
        // Only for an available audio mixer; the same handle as the track.
        trackDesc.insert(QStringLiteral("gain"), QVariantMap{{kOptTarget, trackId}, {QStringLiteral("value"), t.value(QStringLiteral("gainDb"))},
                                                             {QStringLiteral("min"), -60.0}, {QStringLiteral("max"), 12.0}, {QStringLiteral("enabled"), true}});
        if (m_stage >= 4) {
            trackDesc.insert(QStringLiteral("pan"), QVariantMap{{kOptTarget, trackId}, {QStringLiteral("value"), t.value(QStringLiteral("pan"))},
                                                                {QStringLiteral("min"), -50}, {QStringLiteral("max"), 50}, {QStringLiteral("enabled"), true}});
        }
    }
    timeline.insert(QStringLiteral("track"), trackDesc);
    if (m_stage >= 4) {
        QVariantMap audioTargets;
        for (auto it = m_audioTargets.cbegin(); it != m_audioTargets.cend(); ++it) {
            const QString track = it.value().toString();
            audioTargets.insert(it.key(), track.isEmpty() ? -1 : nativeId(track));
        }
        timeline.insert(QStringLiteral("targets"), QVariantMap{{QStringLiteral("video"), m_videoTarget.isEmpty() ? -1 : nativeId(m_videoTarget)},
                                                               {QStringLiteral("audio"), audioTargets}});
    }
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
        QVariantMap trim{{kOptTarget, QStringLiteral("trim-") + m_selectedClip},
                         {QStringLiteral("clip"), nativeId(m_selectedClip)},
                         {QStringLiteral("clips"), clips},
                         {QStringLiteral("tracks"), tracks},
                         {QStringLiteral("modes"), kQualifiedTrimModes}};
        if (m_stage >= 4) {
            const QStringList following = rippleScopeClips();
            QVariantList rippleClips;
            for (const auto &f : following) {
                rippleClips << nativeId(f);
            }
            trim.insert(QStringLiteral("modes"), rippleAvailable() ? kStage4TrimModes : QStringList{QStringLiteral("resize"), QStringLiteral("slip")});
            trim.insert(QStringLiteral("rippleScope"), QVariantMap{{QStringLiteral("clips"), rippleClips},
                                                                   {QStringLiteral("tracks"), tracks},
                                                                   {QStringLiteral("guides"), false}});
        }
        timeline.insert(QStringLiteral("trim"), trim);
        if (gainTargets == 1) {
            timeline.insert(QStringLiteral("clipGain"), QVariantMap{{kOptTarget, QStringLiteral("gain-") + gainClip}, {QStringLiteral("clip"), nativeId(gainClip)},
                                                                    {QStringLiteral("unit"), QStringLiteral("dB")},
                                                                    {QStringLiteral("policy"), QStringLiteral("single_keyframe_existing_effect")}});
        }
    }
    if (m_stage >= 4) {
        const QStringList nudge = nudgeClips();
        if (!nudge.isEmpty() && nudge.size() <= 64) {
            QVariantList clips;
            for (const auto &id : nudge) {
                clips << nativeId(id);
            }
            timeline.insert(QStringLiteral("nudge"), QVariantMap{{kOptTarget, QStringLiteral("nudge-") + nudge.join(QLatin1Char('-'))},
                                                                 {QStringLiteral("clips"), clips},
                                                                 {QStringLiteral("units"), QStringList{QStringLiteral("frame"), QStringLiteral("second")}}});
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
    if (m_stage >= 4) {
        l << kCmdEffectAdd << kCmdEffectSet << kCmdEffectMove << kCmdEffectRemove << kCmdStackSet << kCmdBinTag << kCmdBinSelect << kCmdBinFilter;
    }
    return l;
}

QVariantMap MockKdenlive::capabilities() const
{
    // Advertised (possible) keys, as Kdenlive does; optional ones appear only when they apply.
    QStringList contextKeys{QStringLiteral("serial"), QStringLiteral("emittedAtMs"), QStringLiteral("epoch"), QStringLiteral("ready"),
                            QStringLiteral("active"), QStringLiteral("dialog"), QStringLiteral("focus"), QStringLiteral("project"),
                            QStringLiteral("sequence"), QStringLiteral("activeMonitor"), QStringLiteral("position"), QStringLiteral("fps"),
                            QStringLiteral("playing"), QStringLiteral("speed"), QStringLiteral("tool"), QStringLiteral("timeline")};
    if (m_stage >= 2) {
        contextKeys << QStringLiteral("effect") << QStringLiteral("param") << QStringLiteral("colorWheel") << QStringLiteral("colorWheels")
                    << QStringLiteral("hoveredColorWheel");
    }
    if (m_stage >= 4) {
        contextKeys << QStringLiteral("bin");
    }
    QVariantMap limits{{QStringLiteral("subscriptions"), kMaxLeases}, {QStringLiteral("pendingKeys"), kMaxPendingKeys},
                       {QStringLiteral("queuedActions"), kMaxQueuedActions}, {QStringLiteral("contextHz"), 30},
                       {QStringLiteral("maximumDelta"), 10000}, {QStringLiteral("editingWriters"), 1}};
    if (m_stage >= 3) {
        limits.insert(QStringLiteral("trimGestureSteps"), m_trimGestureSteps);
    }
    if (m_stage >= 4) {
        limits.insert(QStringLiteral("nudgeSelectionClips"), 64);
        limits.insert(QStringLiteral("rippleFollowingClips"), 64);
        limits.insert(QStringLiteral("binSelectionItems"), 256);
        limits.insert(QStringLiteral("binFilterItems"), 2000);
        limits.insert(QStringLiteral("effectStackItems"), 64);
        limits.insert(QStringLiteral("effectParameterHandles"), 32);
    }
    return ok({{QStringLiteral("version"), uint(kVersion)},
               {QStringLiteral("revision"), uint(kRevision)},
               {QStringLiteral("implementation"), QStringLiteral("control-surface mock (stage %1)").arg(m_stage)},
               {QStringLiteral("controls"), controlsForStage()},
               {QStringLiteral("commands"), commandsForStage()},
               {QStringLiteral("contextKeys"), contextKeys},
               {QStringLiteral("limits"), limits},
               {QStringLiteral("trimModes"), m_stage >= 4 ? kStage4TrimModes : m_stage >= 3 ? kQualifiedTrimModes : QStringList{}}});
    // controlDescriptors is optional and absent in Kdenlive's qualified host.
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

QStringList MockKdenlive::candidateActions()
{
    QStringList ids;
    for (const auto &a : catalog::actions()) {
        ids << a.id;
    }
    for (const QString &e : kMockEffects) {
        ids << QStringLiteral("effect_") + e;
    }
    return ids;
}

QList<QVariantMap> MockKdenlive::actionList() const
{
    QList<QVariantMap> list;
    const QString tool = m_context.value(QStringLiteral("tool")).toString() + QStringLiteral("_tool");
    const QString mode = m_editMode + QStringLiteral("_mode");
    for (const auto &a : catalog::actions()) {
        const QString id = a.id;
        list << QVariantMap{{QStringLiteral("id"), id},
                            {QStringLiteral("text"), a.text},
                            {QStringLiteral("enabled"), actionRefusal(id, nullptr).isEmpty()},
                            {QStringLiteral("checkable"), a.checkable},
                            {QStringLiteral("checked"), a.checkable && (tool == id || mode == id)},
                            {QStringLiteral("shortcut"), a.shortcut}};
    }
    for (const QString &e : kMockEffects) {
        const QString id = QStringLiteral("effect_") + e;
        list << QVariantMap{{QStringLiteral("id"), id},
                            {QStringLiteral("text"), e},
                            {QStringLiteral("enabled"), actionRefusal(id, nullptr).isEmpty()},
                            {QStringLiteral("checkable"), false},
                            {QStringLiteral("checked"), false},
                            {QStringLiteral("shortcut"), QString()}};
    }
    return list;
}

QVariantMap MockKdenlive::listActions() const
{
    return ok({{QStringLiteral("actions"), QVariant::fromValue(actionList())}});
}

QVariantMap MockKdenlive::actionRefusal(const QString &id, const Caller *caller) const
{
    // Host context restrictions (k23-contract-mr1a-actions.md); these make the
    // descriptor's "enabled" false.
    const QString focus = m_context.value(QStringLiteral("focus")).toString();
    if (id == QLatin1String("delete_timeline_clip") && focus != QLatin1String("timeline")) {
        return fail(err::TargetNotFound, QStringLiteral("delete needs focus inside the current timeline"));
    }
    if (id == QLatin1String("insert_to_in_point") || id == QLatin1String("overwrite_to_in_point")) {
        if (!m_sourceOpen) {
            return fail(err::TargetNotFound, QStringLiteral("no clip monitor source"));
        }
        bool targeted = false;
        for (const auto &t : m_tracks) {
            targeted = targeted || t.toMap().value(QStringLiteral("target")).toBool();
        }
        if (!targeted) {
            return fail(err::TargetNotFound, QStringLiteral("no timeline target track"));
        }
    }
    if (id.startsWith(QLatin1String("add_marker_guide_")) && focus == QLatin1String("clipMonitor") && !m_sourceOpen) {
        return fail(err::TargetNotFound, QStringLiteral("no clip for a marker"));
    }
    // MR1b-A (k23-mr1b-a-contract.md). Copy and group/ungroup need timeline focus;
    // source-zone insertion an active clip monitor source.
    if ((id == QLatin1String("edit_copy") || id == QLatin1String("group_clip") || id == QLatin1String("ungroup_clip")) && focus != QLatin1String("timeline")) {
        return fail(err::TargetNotFound, QStringLiteral("%1 needs timeline focus").arg(id));
    }
    if (id == QLatin1String("insert_project_tree") && !m_sourceOpen) {
        return fail(err::TargetNotFound, QStringLiteral("no clip monitor source interval"));
    }
    const QString family = catalog::actionFamily(id);
    if (family == QLatin1String("camera")) {
        // Multicam tool only; the camera must exist (no clamping to another one).
        if (m_context.value(QStringLiteral("tool")).toString() != QLatin1String("multicam")) {
            return fail(err::ActionDisabled, QStringLiteral("cameras need the Multicam tool"));
        }
        int videoTracks = 0;
        for (const auto &t : m_tracks) {
            videoTracks += t.toMap().value(QStringLiteral("type")).toString() != QLatin1String("audio");
        }
        if (id.mid(15).toInt() > videoTracks) {
            return fail(err::TargetNotFound, QStringLiteral("no such camera"));
        }
    } else if (family == QLatin1String("layout") && id.mid(11).toInt() > kMockLayoutSlots) {
        return fail(err::ActionDisabled, QStringLiteral("empty layout slot"));
    } else if (family == QLatin1String("tag")) {
        if (id.mid(4).toInt() > kMockProjectTags) {
            return fail(err::UnknownAction, QStringLiteral("no such project tag"));
        }
        if (focus != QLatin1String("bin")) {
            return fail(err::TargetNotFound, QStringLiteral("tags need exactly one selected bin clip"));
        }
    } else if (family == QLatin1String("effect")) {
        if (!kMockEffects.contains(id.mid(7))) {
            return fail(err::UnknownAction, QStringLiteral("no such effect"));
        }
        if (focus != QLatin1String("timeline") && focus != QLatin1String("bin") && focus != QLatin1String("effectStack")) {
            return fail(err::TargetNotFound, QStringLiteral("no compatible effect owner"));
        }
    }
    if (id == QLatin1String("edit_undo") && m_history.isEmpty()) {
        return fail(err::ActionDisabled, QStringLiteral("nothing to undo"));
    }
    if (id == QLatin1String("edit_redo") && m_redo.isEmpty()) {
        return fail(err::ActionDisabled, QStringLiteral("nothing to redo"));
    }
    if (kPlaybackActions.contains(id) && playbackBlocked(QString())) {
        // Listed disabled; invoking it is busy. Pause stays available.
        return fail(err::Busy, QStringLiteral("project monitor trimming preview refuses playback; use select_tool"));
    }
    if (!caller) {
        return {};
    }
    // Checked when invoked: writer ownership, native drag, Slip preview.
    if (isEditingAction(id)) {
        for (const auto &g : m_gestures) {
            if (g.owner != caller->owner) {
                return fail(err::Busy, QStringLiteral("another caller owns the current editing gesture"));
            }
        }
        if (m_dragging) {
            return fail(err::Busy, QStringLiteral("native drag in progress"));
        }
    }
    return {};
}

bool MockKdenlive::playbackBlocked(const QString &monitor) const
{
    const QString m = monitor.isEmpty() || monitor == QLatin1String("active") ? m_context.value(QStringLiteral("activeMonitor")).toString() : monitor;
    return m_trimming && m == QLatin1String("project");
}

QStringList MockKdenlive::editingActions()
{
    return kEditingActions;
}

QStringList MockKdenlive::playbackActions()
{
    return kPlaybackActions;
}

void MockKdenlive::setTrimmingPreview(bool on)
{
    if (m_trimming != on) {
        m_trimming = on;
        bumpSerial(false);  // also announces the enabled change (ActionsChanged)
    }
}

void MockKdenlive::applyAction(const QString &id)
{
    auto transport = [this](int shuttle, bool playing) {
        m_shuttle = shuttle;
        m_context.insert(QStringLiteral("playing"), playing);
        m_context.insert(QStringLiteral("speed"), (m_shuttle < 0 ? -1 : 1) * kShuttleSpeeds[std::abs(m_shuttle)]);
        bumpSerial(false);
    };
    if (id.endsWith(QLatin1String("_tool"))) {
        setContextValue(QStringLiteral("tool"), id.chopped(5));
    } else if (id.endsWith(QLatin1String("_mode")) && id != QLatin1String("perform_multitrack_mode")) {
        m_editMode = id.chopped(5);
    } else if (id == QLatin1String("monitor_play")) {
        const bool playing = m_context.value(QStringLiteral("playing")).toBool();
        transport(playing ? 0 : 1, !playing);
    } else if (id == QLatin1String("monitor_pause")) {
        transport(0, false);  // pauses whatever the cached state says, also while trimming
    } else if (id == QLatin1String("monitor_seek_backward") || id == QLatin1String("monitor_seek_forward")) {
        transport(id.endsWith(QLatin1String("backward")) ? -1 : 1, true);
    } else if (kPlaybackStart.contains(id)) {
        transport(1, true);
    } else if (id == QLatin1String("seek_start")) {
        setPosition(0);
    } else if (id == QLatin1String("seek_end")) {
        setPosition(m_duration);
    } else if (id == QLatin1String("seek_zone_start")) {
        setPosition(m_zoneIn);
    } else if (id == QLatin1String("seek_zone_end")) {
        setPosition(qMax(m_zoneIn, m_zoneOut - 1));
    } else if (id == QLatin1String("mark_in")) {
        m_zoneIn = m_position;
    } else if (id == QLatin1String("mark_out")) {
        m_zoneOut = m_position + 1;  // native exclusive end
    } else if (id == QLatin1String("edit_undo")) {
        m_redo << m_history.takeLast();
        finishAllGestures();
        bumpSerial(true);
    } else if (id == QLatin1String("edit_redo")) {
        m_history << m_redo.takeLast();
        scheduleActionsCheck();
        finishAllGestures();
        bumpSerial(true);
    } else if (isEditingAction(id)) {
        m_redo.clear();
        addUnrelatedHistory(id);  // one native undo entry; history changes start a new epoch
    } else if (id.startsWith(QLatin1String("select")) || id == QLatin1String("deselect_timeline_clip")) {
        bumpSerial(true);  // selection is a target change
    }
    scheduleActionsCheck();
}

void MockKdenlive::scheduleActionsCheck()
{
    if (m_actionsCheckScheduled) {
        return;
    }
    m_actionsCheckScheduled = true;
    QTimer::singleShot(0, this, [this] {
        m_actionsCheckScheduled = false;
        const QList<QVariantMap> now = actionList();
        if (now == m_actionsSent) {
            return;
        }
        m_actionsSent = now;
        for (const Lease &l : std::as_const(m_leases)) {
            sendTo(l.caller, QStringLiteral("ActionsChanged"), {});
        }
    });
}

void MockKdenlive::announceActionsChanged()
{
    for (const Lease &l : std::as_const(m_leases)) {
        sendTo(l.caller, QStringLiteral("ActionsChanged"), {});
    }
}

void MockKdenlive::setSourceOpen(bool on)
{
    m_sourceOpen = on;
    scheduleActionsCheck();
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
    if (control == kNudge) {
        return timeline.value(QStringLiteral("nudge")).toMap().value(kOptTarget).toString();
    }
    if (control == kPan) {
        return timeline.value(QStringLiteral("track")).toMap().value(QStringLiteral("pan")).toMap().value(kOptTarget).toString();
    }
    if (control == kBinRating || control == kCmdBinTag) {
        return m_context.value(QStringLiteral("bin")).toMap().value(QStringLiteral("selection")).toMap().value(kOptTarget).toString();
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
        if (control == kTimelineTarget) {
            return fail(err::InvalidArguments, QStringLiteral("Use integral steps and explicit video/audio kind."), QStringLiteral("delta"));
        }
        if (control == kPan) {
            return fail(err::InvalidArguments, QStringLiteral("Pan requires integral steps."), QStringLiteral("delta"));
        }
        if (control == kNudge) {
            return fail(err::InvalidArguments, QStringLiteral("Nudge requires integral frame/second steps and its selection target."), QStringLiteral("delta"));
        }
        if (control == kEffectFocus) {
            return fail(err::InvalidArguments, QStringLiteral("Effect focus requires bounded integral steps."), QStringLiteral("delta"));
        }
        if (control == kBinCursor) {
            return fail(err::InvalidArguments, QStringLiteral("Bin controls require bounded integral steps."), QStringLiteral("delta"));
        }
        if (control == kTrim) {
            return fail(err::InvalidArguments, QStringLiteral("Use an explicit edge and integral frame delta."), QStringLiteral("delta"));
        }
        return fail(err::InvalidArguments, QStringLiteral("delta must be integral"), QStringLiteral("delta"));
    }
    if (std::abs(delta) > d->maxDelta) {
        if (control == kEffectFocus) {
            return fail(err::InvalidArguments, QStringLiteral("Effect focus requires bounded integral steps."), QStringLiteral("delta"));
        }
        if (control == kBinCursor) {
            return fail(err::InvalidArguments, QStringLiteral("Bin controls require bounded integral steps."), QStringLiteral("delta"));
        }
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
        || !oneOf(QStringLiteral("unit"), {QStringLiteral("frame"), QStringLiteral("second")}, &e)
        || !oneOf(QStringLiteral("kind"), {QStringLiteral("video"), QStringLiteral("audio")}, &e)
        || !oneOf(kOptPhase, {QStringLiteral("update"), QStringLiteral("end"), QStringLiteral("cancel")}, &e)) {
        return e;
    }
    if (options.contains(QStringLiteral("scrub")) && options.value(QStringLiteral("scrub")).typeId() != QMetaType::Bool) {
        return fail(err::InvalidArguments, QStringLiteral("scrub must be boolean"), QStringLiteral("scrub"));
    }
    if (options.contains(QStringLiteral("extend")) && options.value(QStringLiteral("extend")).typeId() != QMetaType::Bool) {
        return fail(err::InvalidArguments, QStringLiteral("extend must be boolean."), QStringLiteral("extend"));
    }
    if (control == kTimelineTarget && !QStringList{QStringLiteral("video"), QStringLiteral("audio")}.contains(options.value(QStringLiteral("kind")).toString())) {
        return fail(err::InvalidArguments, QStringLiteral("Use integral steps and explicit video/audio kind."), QStringLiteral("kind"));
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
        } else if (control == kParamNudge) {
            const QVariantMap param = findParamTarget(target);
            if (target.isEmpty() || param.isEmpty()) {
                return fail(err::TargetNotFound, QStringLiteral("unknown or stale target"), kOptTarget);
            }
            semantic->insert(QStringLiteral("paramName"), param.value(QStringLiteral("name")));
        } else if (control == kAudioGain) {
            if (target.isEmpty() || gainTrack(target).isEmpty()) {
                return fail(err::TargetNotFound, QStringLiteral("unknown or stale gain target"), kOptTarget);
            }
        } else if (control == kPan) {
            if (target.isEmpty() || target != targetFor(control)) {
                return fail(err::TargetNotFound, QStringLiteral("unknown or stale target"), kOptTarget);
            }
        } else if (control == kNudge) {
            if (target.isEmpty() || target != targetFor(control)) {
                return fail(err::InvalidArguments, QStringLiteral("Nudge requires integral frame/second steps and its selection target."), kOptTarget);
            }
        } else if (control == kBinRating) {
            if (!m_binVisible) {
                return fail(err::NotReady, QStringLiteral("No visible native bin view."));
            }
            if (target.isEmpty() || target != targetFor(control)) {
                return fail(err::TargetNotFound, QStringLiteral("Use the current explicit bin selection target."), kOptTarget);
            }
        } else if (target.isEmpty() || target != targetFor(control)) {
            return fail(err::TargetNotFound, QStringLiteral("unknown or stale target"), kOptTarget);
        }
        if (control == kTrim) {
            // mode is required; only the scope's advertised modes are qualified.
            const QStringList modes = m_context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("trim")).toMap().value(QStringLiteral("modes")).toStringList();
            const QString mode = options.value(QStringLiteral("mode")).toString();
            if (!modes.contains(mode)) {
                return fail(err::UnsupportedMode, QStringLiteral("Use an advertised trim mode."), QStringLiteral("mode"));
            }
            if (mode != QLatin1String("slip") && !QStringList{QStringLiteral("start"), QStringLiteral("end")}.contains(options.value(QStringLiteral("edge")).toString())) {
                return fail(err::InvalidArguments, QStringLiteral("Use an explicit edge and integral frame delta."), QStringLiteral("edge"));
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
    if (g.control == kPan) {
        const QString id = g.param.section(QLatin1Char(':'), 1);
        return m_tracks.value(id).toMap().value(QStringLiteral("pan"));
    }
    if (g.control == kTrim || g.control == kNudge) {
        QVariantMap bounds;
        for (auto it = m_clips.cbegin(); it != m_clips.cend(); ++it) {
            const QString id = it.key();
            const QVariantMap c = m_clips.value(id).toMap();
            bounds.insert(id, QVariantList{c.value(QStringLiteral("start")), c.value(QStringLiteral("end")), c.value(QStringLiteral("srcIn")),
                                           c.value(QStringLiteral("srcOut")), c.value(QStringLiteral("minStart")), c.value(QStringLiteral("maxEnd"))});
        }
        return bounds;
    }
    if (g.control == kBinRating) {
        QVariantMap ratings;
        for (const QString &id : m_binSelection) {
            for (const auto &v : m_binClips) {
                const QVariantMap c = v.toMap();
                if (c.value(QStringLiteral("id")).toString() == id) {
                    ratings.insert(id, c.value(QStringLiteral("rating")));
                }
            }
        }
        return ratings;
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
    } else if (g.control == kPan) {
        const QString id = g.param.section(QLatin1Char(':'), 1);
        QVariantMap item = m_tracks.value(id).toMap();
        item.insert(QStringLiteral("pan"), v);
        m_tracks.insert(id, item);
    } else if (g.control == kTrim || g.control == kNudge) {
        const QVariantMap bounds = v.toMap();
        for (auto it = bounds.cbegin(); it != bounds.cend(); ++it) {
            QVariantMap c = m_clips.value(it.key()).toMap();
            c.insert(QStringLiteral("start"), it.value().toList().value(0));
            c.insert(QStringLiteral("end"), it.value().toList().value(1));
            c.insert(QStringLiteral("srcIn"), it.value().toList().value(2));
            c.insert(QStringLiteral("srcOut"), it.value().toList().value(3));
            c.insert(QStringLiteral("minStart"), it.value().toList().value(4));
            c.insert(QStringLiteral("maxEnd"), it.value().toList().value(5));
            m_clips.insert(it.key(), c);
        }
    } else if (g.control == kBinRating) {
        const QVariantMap ratings = v.toMap();
        for (int i = 0; i < m_binClips.size(); ++i) {
            QVariantMap c = m_binClips.at(i).toMap();
            const QString id = c.value(QStringLiteral("id")).toString();
            if (ratings.contains(id)) {
                c.insert(QStringLiteral("rating"), ratings.value(id));
                m_binClips[i] = c;
            }
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

QString MockKdenlive::panTrack(const QString &target) const
{
    const QVariantMap timeline = m_context.value(QStringLiteral("timeline")).toMap();
    const QVariantMap pan = timeline.value(QStringLiteral("track")).toMap().value(QStringLiteral("pan")).toMap();
    return !target.isEmpty() && pan.value(kOptTarget).toString() == target ? target : QString();
}

QString MockKdenlive::gestureSubject(const Pending &p) const
{
    if (p.control == kParamNudge) {
        return p.semantic.value(QStringLiteral("paramName"), m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name"))).toString();
    }
    if (p.control == kColorWheel) {
        return p.semantic.value(QStringLiteral("wheel")).toString();
    }
    if (p.control == kAudioGain) {
        QString clip;
        const QString track = gainTrack(p.target, &clip);
        return clip.isEmpty() ? QStringLiteral("track:") + track : QStringLiteral("clip:") + clip;
    }
    if (p.control == kPan) {
        return QStringLiteral("track:") + panTrack(p.target);
    }
    if (p.control == kTrim) {
        QStringList clips;
        for (const auto &id : m_context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("trim")).toMap().value(QStringLiteral("clips")).toList()) {
            clips << QStringLiteral("clip-%1").arg(id.toInt());
        }
        return clips.join(QLatin1Char(','));
    }
    if (p.control == kNudge) {
        return nudgeClips().join(QLatin1Char(','));
    }
    if (p.control == kBinRating) {
        return m_binSelection.join(QLatin1Char(','));
    }
    return {};
}

QVariantMap MockKdenlive::preflightEdit(const Pending &p, int *frame) const
{
    auto refuse = [](const QString &code, const QString &message) { return QVariantMap{{QStringLiteral("code"), code}, {QStringLiteral("message"), message}}; };
    const bool playing = m_context.value(QStringLiteral("playing")).toBool();
    const bool create = p.semantic.value(QStringLiteral("keyframe")).toString() == QLatin1String("create");
    if (p.control == kParamNudge) {
        const QVariantMap param = findParamTarget(p.target);
        const QString name = p.semantic.value(QStringLiteral("paramName"), param.value(QStringLiteral("name"))).toString();
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
    } else if (p.control == kPan) {
        const QString track = panTrack(p.target);
        const QVariantMap t = m_tracks.value(track).toMap();
        if (track.isEmpty() || t.value(QStringLiteral("type")).toString() != QLatin1String("audio") || t.value(QStringLiteral("lock")).toBool()) {
            return refuse(err::TrackLocked, QStringLiteral("An unlocked audio track in the active mixer is required."));
        }
        if (t.value(QStringLiteral("mute")).toBool()) {
            return refuse(err::NotReady, QStringLiteral("The mixer is muted, monitoring or recording."));
        }
    } else if (p.control == kTrim) {
        const QVariantList tracks = m_context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("trim")).toMap().value(QStringLiteral("tracks")).toList();
        for (const auto &t : tracks) {
            if (m_tracks.value(QStringLiteral("trk-%1").arg(t.toInt())).toMap().value(QStringLiteral("lock")).toBool()) {
                return refuse(err::TrackLocked, QStringLiteral("a track of the edit scope is locked"));
            }
        }
    } else if (p.control == kNudge) {
        const QStringList clips = nudgeClips();
        if (clips.isEmpty() || clips.size() > 64) {
            return refuse(err::ResourceLimit, QStringLiteral("Nudge requires1..64 explicit clip members."));
        }
        for (const auto &id : clips) {
            if (!m_clips.contains(id)) {
                return refuse(err::TargetNotFound, QStringLiteral("A declared clip was removed."));
            }
            if (m_clips.value(id).toMap().value(QStringLiteral("mixed")).toBool()) {
                return refuse(err::UnsupportedMode, QStringLiteral("Nudging mixed clips is not qualified."));
            }
        }
    } else if (p.control == kBinRating) {
        if (!m_binVisible) {
            return refuse(err::NotReady, QStringLiteral("No visible native bin view."));
        }
        if (!validBinSelection() || p.target != binSelectionTarget()) {
            return refuse(err::TargetNotFound, QStringLiteral("Use the current explicit bin selection target."));
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
        if (qBound(-7, m_shuttle + steps, 7) != 0 && playbackBlocked(p.semantic.value(QStringLiteral("monitor")).toString())) {
            // Native forward/rewind refuse the monitor trimming preview; zero still pauses.
            return error(err::Busy, QStringLiteral("project monitor trimming preview refuses playback; use select_tool"));
        }
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
    } else if (p.control == kTimelineTarget) {
        const QString kind = p.semantic.value(QStringLiteral("kind")).toString();
        QString stream;
        QString current;
        QStringList order;
        for (const auto &id : m_trackOrder) {
            const bool audio = m_tracks.value(id).toMap().value(QStringLiteral("type")).toString() == QLatin1String("audio");
            if ((kind == QLatin1String("audio")) == audio) {
                order << id;
            }
        }
        if (kind == QLatin1String("video")) {
            current = m_videoTarget;
            if (current.isEmpty()) {
                return error(err::TargetNotFound, QStringLiteral("Assign a target track first."));
            }
        } else {
            QStringList streams = m_audioTargets.keys();
            streams.sort();
            if (streams.isEmpty()) {
                return error(err::TargetNotFound, QStringLiteral("Assign a source audio stream first."));
            }
            stream = streams.first();
            current = m_audioTargets.value(stream).toString();
            if (current.isEmpty()) {
                return error(err::TargetNotFound, QStringLiteral("Assign a target track first."));
            }
        }
        const int idx = order.indexOf(current);
        if (idx < 0) {
            return error(err::TargetNotFound, QStringLiteral("Assign a target track first."));
        }
        const QString dest = order.value(qBound(0, idx + steps, int(order.size()) - 1));
        if (kind == QLatin1String("audio") && dest != current) {
            for (auto it = m_audioTargets.cbegin(); it != m_audioTargets.cend(); ++it) {
                if (it.key() != stream && it.value().toString() == dest) {
                    return error(err::EditFailed, QStringLiteral("The destination already carries another source stream."));
                }
            }
        }
        *changed = dest != current;
        if (*changed) {
            if (kind == QLatin1String("video")) {
                m_videoTarget = dest;
            } else {
                m_audioTargets.insert(stream, dest);
            }
            refreshDescriptors();
            bumpSerial(true);
        }
        st = {{QStringLiteral("track"), nativeId(dest)}, {QStringLiteral("kind"), kind}};
    } else if (p.control == kParamFocus) {
        const QString cur = m_context.value(QStringLiteral("param")).toMap().value(QStringLiteral("name")).toString();
        const int idx = qBound(0, int(m_paramOrder.indexOf(cur)) + steps, int(m_paramOrder.size()) - 1);
        *changed = m_paramOrder.value(idx) != cur;
        if (*changed) {
            finishAllGestures();
            setContextValue(QStringLiteral("param"), paramDescriptor(m_paramOrder.at(idx)));
        }
        st = {{QStringLiteral("param"), m_paramOrder.value(idx)}};
    } else if (p.control == kEffectFocus) {
        if (!m_effectStackShown || m_effectStack.isEmpty() || m_focusedEffect < 0) {
            return error(err::TargetNotFound, QStringLiteral("No effect is shown."));
        }
        const int idx = qBound(0, m_focusedEffect + steps, int(m_effectStack.size()) - 1);
        const QVariantMap item = m_effectStack.value(idx).toMap();
        if (item.value(QStringLiteral("group")).toBool()) {
            return error(err::UnsupportedParameter, QStringLiteral("Nested effect groups are not focus targets."));
        }
        *changed = idx != m_focusedEffect;
        if (*changed) {
            m_focusedEffect = idx;
            m_focusedEffectParam = QStringLiteral("rOffset");
            refreshDescriptors();
            bumpSerial(true);
        }
        st = {{QStringLiteral("index"), m_focusedEffect}};
    } else if (p.control == kBinCursor) {
        if (!m_binVisible) {
            return error(err::NotReady, QStringLiteral("No visible native bin view."));
        }
        const QStringList visible = visibleBinClipIds();
        if (visible.isEmpty()) {
            return error(err::TargetNotFound, QStringLiteral("The bin view is empty."));
        }
        const int before = m_binCursor;
        m_binCursor = qBound(0, m_binCursor + steps, int(visible.size()) - 1);
        *changed = before != m_binCursor;
        if (*changed) {
            const QString id = visible.value(m_binCursor);
            if (p.semantic.value(QStringLiteral("extend")).toBool()) {
                if (!m_binSelection.contains(id)) {
                    m_binSelection << id;
                }
            } else {
                m_binSelection = {id};
            }
            refreshDescriptors();
            bumpSerial(true);
        }
        st = {};
    } else if (p.control == kParamNudge) {
        const Gesture &g = m_gestures[gk];
        const QString name = g.param;
        // The bound handle's native units: MR2's focused param (0..100, step 1,
        // fine 0.1) or an effect.params handle (e.g. an offset, -1..1 by 0.01).
        const QVariantMap desc = findParamTarget(p.target);
        const double fine = desc.value(QStringLiteral("fineStep"), 0.1).toDouble();
        const double step = p.semantic.value(QStringLiteral("step")).toString() == QLatin1String("fine") ? fine : desc.value(QStringLiteral("step"), 1.0).toDouble();
        const double lo = desc.value(QStringLiteral("min"), 0.0).toDouble(), hi = desc.value(QStringLiteral("max"), 100.0).toDouble();
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
        const double scale = std::round(1.0 / fine);  // round to the fine step (10 for 0.1)
        const double v = qBound(lo, std::round((before + p.delta * step) * scale) / scale, hi);
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
    } else if (p.control == kPan) {
        const QString id = m_gestures[gk].param.section(QLatin1Char(':'), 1);
        QVariantMap item = m_tracks.value(id).toMap();
        const int before = item.value(QStringLiteral("pan")).toInt();
        const int v = qBound(-50, before + steps, 50);
        item.insert(QStringLiteral("pan"), v);
        m_tracks.insert(id, item);
        *changed = v != before;
        st = {{QStringLiteral("pan"), v}};
        refreshDescriptors();
        bumpSerial(false);
    } else if (p.control == kNudge) {
        Gesture &g = m_gestures[gk];
        if (steps != 0 && g.steps >= m_trimGestureSteps) {
            return error(err::ResourceLimit, QStringLiteral("End this gesture before starting more native edit steps."));
        }
        const QStringList clips = m_gestures[gk].param.split(QLatin1Char(','), Qt::SkipEmptyParts);
        const QVariantMap fps = m_context.value(QStringLiteral("fps")).toMap();
        const double fpsValue = fps.value(QStringLiteral("den")).toDouble() == 0.0 ? 25.0 : fps.value(QStringLiteral("num")).toDouble() / fps.value(QStringLiteral("den")).toDouble();
        const int frames = p.semantic.value(QStringLiteral("unit")).toString() == QLatin1String("second") ? int(std::llround(p.delta * fpsValue)) : steps;
        for (const auto &id : clips) {
            const QVariantMap c = m_clips.value(id).toMap();
            const int start = c.value(QStringLiteral("start")).toInt() + frames;
            const int end = c.value(QStringLiteral("end")).toInt() + frames;
            if (start < 0) {
                return error(err::EditFailed, QStringLiteral("The requested frame range is outside the native bounds."));
            }
            for (auto it = m_clips.cbegin(); it != m_clips.cend(); ++it) {
                if (clips.contains(it.key())) {
                    continue;
                }
                const QVariantMap other = it.value().toMap();
                if (other.value(QStringLiteral("track")).toString() == c.value(QStringLiteral("track")).toString()
                    && start < other.value(QStringLiteral("end")).toInt() && end > other.value(QStringLiteral("start")).toInt()) {
                    return error(err::EditFailed, QStringLiteral("The requested nudge collides with an undeclared clip."));
                }
            }
        }
        for (const auto &id : clips) {
            QVariantMap c = m_clips.value(id).toMap();
            c.insert(QStringLiteral("start"), c.value(QStringLiteral("start")).toInt() + frames);
            c.insert(QStringLiteral("end"), c.value(QStringLiteral("end")).toInt() + frames);
            c.insert(QStringLiteral("minStart"), c.value(QStringLiteral("minStart")).toInt() + frames);
            c.insert(QStringLiteral("maxEnd"), c.value(QStringLiteral("maxEnd")).toInt() + frames);
            m_clips.insert(id, c);
        }
        *changed = frames != 0;
        if (*changed) {
            ++g.steps;
        }
        st = {{QStringLiteral("frames"), frames}};
        refreshDescriptors();
        bumpSerial(false);
    } else if (p.control == kTrim) {
        Gesture &g = m_gestures[gk];
        if (steps != 0 && g.steps >= m_trimGestureSteps) {
            return error(err::ResourceLimit, QStringLiteral("End this gesture before starting more native edit steps."));
        }
        const QStringList clips = selectedTrimClips();
        const QString mode = p.semantic.value(QStringLiteral("mode")).toString();
        const bool atEnd = p.semantic.value(QStringLiteral("edge")).toString() == QLatin1String("end");
        int applied = steps;
        if (mode == QLatin1String("slip")) {
            for (const auto &id : clips) {
                const QVariantMap c = m_clips.value(id).toMap();
                const int in = c.value(QStringLiteral("srcIn")).toInt() - steps;
                const int out = c.value(QStringLiteral("srcOut")).toInt() - steps;
                if (in < 0 || out > c.value(QStringLiteral("sourceLength")).toInt()) {
                    return error(err::EditFailed, QStringLiteral("The requested slip exceeds the source interval."));
                }
            }
            for (const auto &id : clips) {
                QVariantMap c = m_clips.value(id).toMap();
                c.insert(QStringLiteral("srcIn"), c.value(QStringLiteral("srcIn")).toInt() - steps);
                c.insert(QStringLiteral("srcOut"), c.value(QStringLiteral("srcOut")).toInt() - steps);
                c.insert(QStringLiteral("minStart"), c.value(QStringLiteral("minStart")).toInt() + steps);
                c.insert(QStringLiteral("maxEnd"), c.value(QStringLiteral("maxEnd")).toInt() + steps);
                m_clips.insert(id, c);
            }
        } else if (mode == QLatin1String("ripple")) {
            const int growth = atEnd ? steps : -steps;
            for (const auto &id : clips) {
                const QVariantMap c = m_clips.value(id).toMap();
                if (c.value(QStringLiteral("end")).toInt() - c.value(QStringLiteral("start")).toInt() + growth < 1) {
                    return error(err::InvalidArguments, QStringLiteral("The requested duration is outside the native frame range."));
                }
                if ((atEnd && c.value(QStringLiteral("end")).toInt() + steps > c.value(QStringLiteral("maxEnd")).toInt())
                    || (!atEnd && c.value(QStringLiteral("start")).toInt() + steps < c.value(QStringLiteral("minStart")).toInt())) {
                    return error(err::EditFailed, QStringLiteral("The exact group resize was refused; original members were restored."));
                }
            }
            const QStringList following = rippleScopeClips();
            QVariantMap next = m_clips;
            for (const auto &id : clips) {
                QVariantMap c = next.value(id).toMap();
                if (atEnd) {
                    c.insert(QStringLiteral("end"), c.value(QStringLiteral("end")).toInt() + steps);
                    c.insert(QStringLiteral("srcOut"), c.value(QStringLiteral("srcOut")).toInt() + steps);
                } else {
                    c.insert(QStringLiteral("end"), c.value(QStringLiteral("end")).toInt() + growth);
                    c.insert(QStringLiteral("srcIn"), c.value(QStringLiteral("srcIn")).toInt() + steps);
                    c.insert(QStringLiteral("minStart"), c.value(QStringLiteral("minStart")).toInt() + growth);
                    c.insert(QStringLiteral("maxEnd"), c.value(QStringLiteral("maxEnd")).toInt() + growth);
                }
                next.insert(id, c);
            }
            for (const auto &id : following) {
                QVariantMap c = next.value(id).toMap();
                c.insert(QStringLiteral("start"), c.value(QStringLiteral("start")).toInt() + growth);
                c.insert(QStringLiteral("end"), c.value(QStringLiteral("end")).toInt() + growth);
                c.insert(QStringLiteral("minStart"), c.value(QStringLiteral("minStart")).toInt() + growth);
                c.insert(QStringLiteral("maxEnd"), c.value(QStringLiteral("maxEnd")).toInt() + growth);
                next.insert(id, c);
            }
            QStringList changedClips = clips + following;
            changedClips.removeDuplicates();
            for (const auto &id : changedClips) {
                const QVariantMap c = next.value(id).toMap();
                if (c.value(QStringLiteral("start")).toInt() < 0) {
                    return error(err::EditFailed, QStringLiteral("The exact group resize was refused; original members were restored."));
                }
                for (auto it = next.cbegin(); it != next.cend(); ++it) {
                    if (changedClips.contains(it.key())) {
                        continue;
                    }
                    const QVariantMap other = it.value().toMap();
                    if (other.value(QStringLiteral("track")).toString() == c.value(QStringLiteral("track")).toString()
                        && c.value(QStringLiteral("start")).toInt() < other.value(QStringLiteral("end")).toInt()
                        && c.value(QStringLiteral("end")).toInt() > other.value(QStringLiteral("start")).toInt()) {
                        return error(err::EditFailed, QStringLiteral("The exact group resize was refused; original members were restored."));
                    }
                }
            }
            m_clips = next;
            applied = growth;
        } else {
            for (const auto &id : clips) {
                const QVariantMap c = m_clips.value(id).toMap();
                const int start = c.value(QStringLiteral("start")).toInt(), end = c.value(QStringLiteral("end")).toInt();
                applied = atEnd ? qBound(start + 1 - end, applied, c.value(QStringLiteral("maxEnd")).toInt() - end)
                                : qBound(c.value(QStringLiteral("minStart")).toInt() - start, applied, end - 1 - start);
            }
            QVariantMap next = m_clips;
            for (const auto &id : clips) {
                QVariantMap c = next.value(id).toMap();
                const QString field = atEnd ? QStringLiteral("end") : QStringLiteral("start");
                c.insert(field, c.value(field).toInt() + applied);
                const QString sourceField = atEnd ? QStringLiteral("srcOut") : QStringLiteral("srcIn");
                c.insert(sourceField, c.value(sourceField).toInt() + applied);
                next.insert(id, c);
            }
            for (const auto &id : clips) {
                const QVariantMap c = next.value(id).toMap();
                for (auto it = next.cbegin(); it != next.cend(); ++it) {
                    if (clips.contains(it.key())) {
                        continue;
                    }
                    const QVariantMap other = it.value().toMap();
                    if (other.value(QStringLiteral("track")).toString() == c.value(QStringLiteral("track")).toString()
                        && c.value(QStringLiteral("start")).toInt() < other.value(QStringLiteral("end")).toInt()
                        && c.value(QStringLiteral("end")).toInt() > other.value(QStringLiteral("start")).toInt()) {
                        return error(err::EditFailed, QStringLiteral("The exact group resize was refused; original members were restored."));
                    }
                }
            }
            m_clips = next;
        }
        *changed = applied != 0;
        if (*changed) {
            ++g.steps;
        }
        const QVariantMap first = m_clips.value(clips.value(0)).toMap();
        if (mode == QLatin1String("slip")) {
            st = {{QStringLiteral("frames"), steps}};
        } else {
            st = {{QStringLiteral("clip"), clips.value(0)},
                  {QStringLiteral("start"), first.value(QStringLiteral("start"))},
                  {QStringLiteral("end"), first.value(QStringLiteral("end"))},
                  {QStringLiteral("duration"), first.value(QStringLiteral("end")).toInt() - first.value(QStringLiteral("start")).toInt()},
                  {QStringLiteral("mode"), mode}};
        }
        refreshDescriptors();
        bumpSerial(false);
    } else if (p.control == kBinRating) {
        for (int i = 0; i < m_binClips.size(); ++i) {
            QVariantMap c = m_binClips.at(i).toMap();
            if (m_binSelection.contains(c.value(QStringLiteral("id")).toString())) {
                const int nativeRating = qBound(0, c.value(QStringLiteral("rating")).toInt() + 2 * steps, 10);
                *changed = *changed || c.value(QStringLiteral("rating")).toInt() != nativeRating;
                c.insert(QStringLiteral("rating"), nativeRating);
                m_binClips[i] = c;
            }
        }
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
                } else if (p.control == kPan) {
                    st.insert(QStringLiteral("pan"), m_tracks.value(p.target).toMap().value(QStringLiteral("pan")));
                } else if (p.control == kTrim) {
                    if (st.contains(QStringLiteral("clip"))) {
                        const QVariantMap c = m_clips.value(st.value(QStringLiteral("clip")).toString()).toMap();
                        st.insert(QStringLiteral("start"), c.value(QStringLiteral("start")));
                        st.insert(QStringLiteral("end"), c.value(QStringLiteral("end")));
                        st.insert(QStringLiteral("duration"), c.value(QStringLiteral("end")).toInt() - c.value(QStringLiteral("start")).toInt());
                    }
                }  // bin.rating: {state, changed} only, as Kdenlive
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
        scheduleActionsCheck();
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
    const bool known = catalog::isOffered(id) && (catalog::actionFamily(id) != QLatin1String("effect") || kMockEffects.contains(id.mid(7)));
    if (!known) {
        // Dialog actions and anything else not curated: never a keyboard-fallback invitation.
        return fail(err::UnknownAction, QStringLiteral("not offered to control surfaces"), QStringLiteral("id"));
    }
    if (m_actions.size() >= kMaxQueuedActions) {
        return fail(err::ResourceLimit, QStringLiteral("too many queued actions"));
    }
    const QVariantMap refusal = actionRefusal(id, &c);
    if (!refusal.isEmpty()) {
        return refusal;
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
        QVariantMap refusal = stateCheck();
        if (refusal.isEmpty()) {
            refusal = actionRefusal(q.id, &q.caller);  // revalidated at dispatch
        }
        if (q.epoch != m_epoch) {
            outcome = fail(err::StaleContext, QStringLiteral("context changed before invocation"));
        } else if (!refusal.isEmpty()) {
            outcome = refusal;
        } else {
            finishAllGestures();  // discrete operations terminate editing gestures first
            m_triggered << q.id;
            applyAction(q.id);
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
    if (value != 0 && playbackBlocked(options.value(QStringLiteral("monitor")).toString())) {
        return fail(err::Busy, QStringLiteral("project monitor trimming preview refuses playback; use select_tool"));
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
    // As Kdenlive: option validation first, then dispatch; an unknown command
    // is unsupported_control with an empty field.
    const QStringList allowed = optionsFor(command);
    int budget = kMaxStringInput - int(command.size());
    QVariantMap e = admit(c, args, allowed, &budget);
    if (!e.isEmpty()) {
        return e;
    }
    if (!commandsForStage().contains(command)) {
        return fail(err::UnsupportedControl, QStringLiteral("command not offered"));
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
            scheduleActionsCheck();
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
            scheduleActionsCheck();
            refreshDescriptors();
            bumpSerial(false);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    auto stackTargetOk = [&] {
        const QString stackTarget = m_context.value(QStringLiteral("effect")).toMap().value(QStringLiteral("stack")).toMap().value(kOptTarget).toString();
        const QString requested = args.value(kOptTarget, stackTarget).toString();
        return m_effectStackShown && !stackTarget.isEmpty() && requested == stackTarget;
    };
    auto effectTargetIndex = [&]() { return effectIndexForTarget(target); };
    if (command == kCmdEffectAdd) {
        const bool hasId = args.contains(QStringLiteral("id"));
        const bool hasPreset = args.contains(QStringLiteral("preset"));
        if (hasId == hasPreset) {
            return fail(err::InvalidArguments, QStringLiteral("Supply exactly one effect id or registered preset."), hasId ? QStringLiteral("preset") : QStringLiteral("id"));
        }
        const QString key = hasId ? QStringLiteral("id") : QStringLiteral("preset");
        if (args.value(key).typeId() != QMetaType::QString) {
            return fail(err::InvalidArguments, QStringLiteral("The repository identifier must be a string."), key);
        }
        const QString id = args.value(key).toString();
        if (id.isEmpty() || id.size() > 128) {
            return fail(err::InvalidArguments, QStringLiteral("Use a bounded repository identifier."), key);
        }
        if (!stackTargetOk()) {
            return fail(err::TargetNotFound, QStringLiteral("The explicit effect/stack handle is stale."), kOptTarget);
        }
        if (!kMockEffects.contains(id) && id != QLatin1String("warm-look")) {
            return fail(err::UnsupportedParameter, hasPreset ? QStringLiteral("The saved preset name is absent or ambiguous.") : QStringLiteral("The effect/preset is not installed."));
        }
        if (m_effectStack.size() >= 64) {
            return fail(err::ResourceLimit, QStringLiteral("The interactive stack limit is64 effects."));
        }
        m_effectStack << effect(hasPreset ? QStringLiteral("preset:") + id : id);
        m_focusedEffect = m_effectStack.size() - 1;
        m_history << QStringLiteral("effect add %1").arg(id);
        refreshDescriptors();
        bumpSerial(true);
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), true}});
    }
    if (command == kCmdEffectSet || command == kCmdEffectMove || command == kCmdEffectRemove) {
        const int idx = effectTargetIndex();
        if (idx < 0) {
            return fail(err::TargetNotFound, QStringLiteral("The explicit effect/stack handle is stale."), kOptTarget);
        }
        QVariantMap item = m_effectStack.at(idx).toMap();
        if (command == kCmdEffectSet) {
            if (args.value(QStringLiteral("value")).typeId() != QMetaType::Bool) {
                return fail(err::InvalidArguments, QStringLiteral("value must be boolean."), QStringLiteral("value"));
            }
            if (args.value(QStringLiteral("what")).toString() != QLatin1String("enabled")) {
                return fail(err::UnsupportedMode, QStringLiteral("Use an advertised effect property."), QStringLiteral("what"));
            }
            const bool value = args.value(QStringLiteral("value")).toBool();
            const bool changed = item.value(QStringLiteral("enabled"), true).toBool() != value;
            item.insert(QStringLiteral("enabled"), value);
            m_effectStack[idx] = item;
            if (changed) {
                m_history << QStringLiteral("effect set %1").arg(target);
                refreshDescriptors();
                bumpSerial(false);
            }
            return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
        }
        if (command == kCmdEffectMove) {
            if (!args.contains(QStringLiteral("delta")) || args.value(QStringLiteral("delta")).typeId() == QMetaType::Bool
                || !args.value(QStringLiteral("delta")).canConvert<double>()) {
                return fail(err::InvalidArguments, QStringLiteral("delta must be numeric."), QStringLiteral("delta"));
            }
            const double d = args.value(QStringLiteral("delta")).toDouble();
            if (!std::isfinite(d) || d != std::trunc(d) || std::abs(d) > 64) {
                return fail(err::InvalidArguments, QStringLiteral("Move requires integral steps within64 rows."), QStringLiteral("delta"));
            }
            if (item.value(QStringLiteral("builtin")).toBool()) {
                return fail(err::UnsupportedParameter, QStringLiteral("Built-in effects and nested groups cannot be reordered here."));
            }
            const int dest = qBound(0, idx + int(d), int(m_effectStack.size()) - 1);
            if (m_effectStack.value(dest).toMap().value(QStringLiteral("builtin")).toBool() || m_effectStack.value(dest).toMap().value(QStringLiteral("group")).toBool()) {
                return fail(err::UnsupportedParameter, QStringLiteral("Built-in effects and nested groups cannot be reordered here."));
            }
            const bool changed = dest != idx;
            if (changed) {
                m_effectStack.removeAt(idx);
                m_effectStack.insert(dest, item);
                m_focusedEffect = dest;
                m_history << QStringLiteral("effect move %1").arg(target);
                refreshDescriptors();
                bumpSerial(true);
            }
            return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
        }
        if (item.value(QStringLiteral("builtin")).toBool()) {
            return fail(err::UnsupportedParameter, QStringLiteral("Built-in effects cannot be removed."));
        }
        m_effectStack.removeAt(idx);
        m_focusedEffect = m_effectStack.isEmpty() ? -1 : qBound(0, idx, int(m_effectStack.size()) - 1);
        m_history << QStringLiteral("effect remove %1").arg(target);
        refreshDescriptors();
        bumpSerial(true);
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), true}});
    }
    if (command == kCmdStackSet) {
        if (!stackTargetOk()) {
            return fail(err::TargetNotFound, QStringLiteral("The explicit effect/stack handle is stale."), kOptTarget);
        }
        if (args.value(QStringLiteral("value")).typeId() != QMetaType::Bool) {
            return fail(err::InvalidArguments, QStringLiteral("value must be boolean."), QStringLiteral("value"));
        }
        const QString what = args.value(QStringLiteral("what")).toString();
        if (what != QLatin1String("enabled") && what != QLatin1String("compare")) {
            return fail(err::UnsupportedMode, QStringLiteral("Use an advertised effect property."), QStringLiteral("what"));
        }
        if (what == QLatin1String("compare") && !m_effectCompareAvailable) {
            return fail(err::UnsupportedMode, QStringLiteral("Compare requires a visible native clip effect stack."));
        }
        const bool value = args.value(QStringLiteral("value")).toBool();
        bool changed = false;
        if (what == QLatin1String("enabled")) {
            changed = m_effectStackEnabled != value;
            m_effectStackEnabled = value;
            if (changed) {
                m_history << QStringLiteral("effect stack enabled");
            }
        } else {
            changed = m_effectCompare != value;
            m_effectCompare = value;
        }
        if (changed) {
            refreshDescriptors();
            bumpSerial(false);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    if (command == kCmdBinTag) {
        if (!m_binVisible) {
            return fail(err::NotReady, QStringLiteral("No visible native bin view."));
        }
        const QString tag = args.value(QStringLiteral("tag")).toString();
        if (!binTagIds().contains(tag)) {
            return fail(err::InvalidArguments, QStringLiteral("Use a current native tag color from bin.tags."), QStringLiteral("tag"));
        }
        if (target.isEmpty() || target != binSelectionTarget()) {
            return fail(err::TargetNotFound, QStringLiteral("The bin selection handle is stale."), kOptTarget);
        }
        if (args.value(QStringLiteral("value")).typeId() != QMetaType::Bool) {
            return fail(err::InvalidArguments, QStringLiteral("A tag and boolean value are required."), QStringLiteral("value"));
        }
        const bool value = args.value(QStringLiteral("value")).toBool();
        bool changed = false;
        for (int i = 0; i < m_binClips.size(); ++i) {
            QVariantMap c = m_binClips.at(i).toMap();
            if (!m_binSelection.contains(c.value(QStringLiteral("id")).toString())) {
                continue;
            }
            QStringList tags = c.value(QStringLiteral("tags")).toStringList();
            if (value && !tags.contains(tag)) {
                tags << tag;
                changed = true;
            } else if (!value && tags.removeAll(tag) > 0) {
                changed = true;
            }
            c.insert(QStringLiteral("tags"), tags);
            m_binClips[i] = c;
        }
        if (changed) {
            m_history << QStringLiteral("bin tag %1").arg(tag);
            refreshDescriptors();
            bumpSerial(false);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    if (command == kCmdBinSelect) {
        if (!m_binVisible) {
            return fail(err::NotReady, QStringLiteral("No visible native bin view."));
        }
        const QString tag = args.value(QStringLiteral("tag")).toString();
        if (tag.isEmpty()) {
            return fail(err::InvalidArguments, QStringLiteral("An explicit tag is required."), QStringLiteral("tag"));
        }
        if (!binTagIds().contains(tag)) {
            return fail(err::InvalidArguments, QStringLiteral("Use a current native tag color from bin.tags."), QStringLiteral("tag"));
        }
        if (m_binClips.size() > 2000) {
            return fail(err::ResourceLimit, QStringLiteral("Interactive bin filtering is bounded to2000 clips."));
        }
        QStringList selected;
        const QStringList visible = visibleBinClipIds();
        for (const auto &v : m_binClips) {
            const QVariantMap c = v.toMap();
            const QString id = c.value(QStringLiteral("id")).toString();
            if (visible.contains(id) && c.value(QStringLiteral("tags")).toStringList().contains(tag)) {
                selected << c.value(QStringLiteral("id")).toString();
            }
        }
        const bool changed = selected != m_binSelection;
        m_binSelection = selected;
        if (changed) {
            refreshDescriptors();
            bumpSerial(true);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    if (command == kCmdBinFilter) {
        if (!m_binVisible) {
            return fail(err::NotReady, QStringLiteral("No visible native bin view."));
        }
        if (m_binClips.size() > 2000) {
            return fail(err::ResourceLimit, QStringLiteral("Interactive bin filtering is bounded to2000 clips."));
        }
        if (args.contains(QStringLiteral("clear")) && args.value(QStringLiteral("clear")).typeId() != QMetaType::Bool) {
            return fail(err::InvalidArguments, QStringLiteral("clear must be boolean."), QStringLiteral("clear"));
        }
        const bool clear = args.value(QStringLiteral("clear")).toBool();
        if (clear && (args.contains(QStringLiteral("tag")) || args.contains(QStringLiteral("rating")))) {
            return fail(err::InvalidArguments, QStringLiteral("clear cannot be combined with filters."), QStringLiteral("clear"));
        }
        if (args.contains(QStringLiteral("rating"))) {
            // Kdenlive (bincontrol.cpp): signed or unsigned 32/64-bit integers in 0..5.
            const QVariant value = args.value(QStringLiteral("rating"));
            const int type = value.typeId();
            const bool signedInteger = type == QMetaType::Int || type == QMetaType::LongLong;
            const bool unsignedInteger = type == QMetaType::UInt || type == QMetaType::ULongLong;
            if ((!signedInteger && !unsignedInteger) || (signedInteger && value.toLongLong() < 0) || value.toULongLong() > 5) {
                return fail(err::InvalidArguments, QStringLiteral("rating is an integer in0..5 stars."), QStringLiteral("rating"));
            }
        }
        const QString tag = args.value(QStringLiteral("tag")).toString();
        if (!tag.isEmpty() && !binTagIds().contains(tag)) {
            return fail(err::InvalidArguments, QStringLiteral("Use a current native tag color from bin.tags."), QStringLiteral("tag"));
        }
        if (!clear && tag.isEmpty() && !args.contains(QStringLiteral("rating"))) {
            return fail(err::InvalidArguments, QStringLiteral("Supply a tag/rating filter or clear."));
        }
        const QStringList oldTags = m_binFilterTags;
        const QVariantList oldRatings = m_binFilterRatings;
        const QVariantList oldTypes = m_binFilterTypes;
        const int oldUsage = m_binFilterUsage;
        if (clear) {
            m_binFilterTags.clear();
            m_binFilterRatings.clear();
            m_binFilterTypes.clear();
            m_binFilterUsage = 0;
        } else {
            m_binFilterTags = tag.isEmpty() ? QStringList{} : QStringList{tag};
            m_binFilterRatings = args.contains(QStringLiteral("rating")) ? QVariantList{args.value(QStringLiteral("rating")).toInt()} : QVariantList{};
        }
        const bool changed = oldTags != m_binFilterTags || oldRatings != m_binFilterRatings || oldTypes != m_binFilterTypes || oldUsage != m_binFilterUsage;
        if (changed) {
            refreshDescriptors();
            bumpSerial(false);
        }
        return ok({{QStringLiteral("state"), QStringLiteral("applied")}, {QStringLiteral("changed"), changed}});
    }
    // track.set, validated in Kdenlive's order: a current track handle (any
    // track's issued handle), a boolean value, soloMode only with solo, then
    // what and its applicability.
    if (command != kCmdTrackSet) {
        return fail(err::UnsupportedControl, QStringLiteral("command not offered"));
    }
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
    const bool routeTarget = m_stage >= 4 && what == QLatin1String("target");
    bool changed = routeTarget ? (audio ? m_audioTargets.values().contains(target) : m_videoTarget == target) != value : t.value(what).toBool() != value;
    t.insert(what, value);
    m_tracks.insert(target, t);
    if (routeTarget && changed) {
        if (audio) {
            for (auto it = m_audioTargets.begin(); it != m_audioTargets.end();) {
                if (it.value().toString() == target) {
                    it = m_audioTargets.erase(it);
                } else {
                    ++it;
                }
            }
            if (value) {
                m_audioTargets.insert(QStringLiteral("0"), target);
            }
        } else {
            m_videoTarget = value ? target : QString();
        }
    }
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
        scheduleActionsCheck();
        refreshDescriptors();
        bumpSerial(routeTarget);
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
    if (key == QLatin1String("tool")) {
        m_trimming = value.toString() == QLatin1String("slip");  // the Slip tool enters the trimming preview here
    }
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

void MockKdenlive::selectClipGroup(const QStringList &clipIds)
{
    for (const auto &id : clipIds) {
        if (!m_clips.contains(id)) {
            return;
        }
    }
    m_nudgeSelection = clipIds;
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::setAudioRouting(const QVariantMap &routing)
{
    m_audioTargets = routing;
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::showBin(bool on)
{
    m_binVisible = on;
    if (on) {
        m_context.insert(QStringLiteral("focus"), QStringLiteral("bin"));
    } else if (m_context.value(QStringLiteral("focus")).toString() == QLatin1String("bin")) {
        m_context.insert(QStringLiteral("focus"), QStringLiteral("timeline"));
    }
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::setBinClips(const QList<QVariantMap> &clips)
{
    m_binClips.clear();
    int i = 0;
    for (QVariantMap c : clips) {
        if (!c.contains(QStringLiteral("id"))) {
            c.insert(QStringLiteral("id"), QStringLiteral("bin-%1").arg(++i));
        }
        if (!c.contains(QStringLiteral("complete"))) {
            c.insert(QStringLiteral("complete"), true);
        }
        m_binClips << c;
    }
    m_binCursor = qBound(0, m_binCursor, qMax(0, m_binClips.size() - 1));
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::selectBinClips(const QStringList &clipIds)
{
    m_binSelection = clipIds;
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::showEffectStack(const QList<QVariantMap> &effects, bool compareAvailable)
{
    m_effectStack.clear();
    for (QVariantMap e : effects) {
        if (!e.contains(QStringLiteral("enabled"))) {
            e.insert(QStringLiteral("enabled"), true);
        }
        if (!e.contains(QStringLiteral("builtin"))) {
            e.insert(QStringLiteral("builtin"), false);
        }
        if (!e.contains(QStringLiteral("group"))) {
            e.insert(QStringLiteral("group"), false);
        }
        m_effectStack << e;
    }
    if (m_effectStack.isEmpty()) {
        m_effectStack << effect(QStringLiteral("lift_gamma_gain"));
    }
    m_effectStackShown = true;
    m_effectCompareAvailable = compareAvailable;
    m_focusedEffect = 0;
    m_focusedEffectParam = QStringLiteral("rOffset");
    m_context.insert(QStringLiteral("focus"), QStringLiteral("effectStack"));
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::hideEffectStack()
{
    m_effectStackShown = false;
    m_context.remove(QStringLiteral("effect"));
    if (m_context.value(QStringLiteral("focus")).toString() == QLatin1String("effectStack")) {
        m_context.insert(QStringLiteral("focus"), QStringLiteral("timeline"));
    }
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::focusEffect(int index)
{
    if (index < 0 || index >= m_effectStack.size()) {
        return;
    }
    m_effectStackShown = true;
    m_focusedEffect = index;
    m_focusedEffectParam = QStringLiteral("rOffset");
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::focusEffectParameter(const QString &name)
{
    if (name.isEmpty() || !m_params.contains(name)) {
        return;
    }
    m_focusedEffectParam = name;
    refreshDescriptors();
    bumpSerial(true);
}

void MockKdenlive::addUnrelatedHistory(const QString &label)
{
    // As Kdenlive: an undo-stack change not made by the interface ends open
    // gestures and starts a new epoch.
    finishAllGestures();
    m_redo.clear();  // a new history entry drops the redo stack
    m_history << label;
    scheduleActionsCheck();
    bumpSerial(true);
}

void MockKdenlive::bumpSerial(bool epoch)
{
    scheduleActionsCheck();
    m_context.insert(kCtxSerial, u64(++m_serial));
    if (epoch) {
        m_context.insert(kCtxEpoch, u64(++m_epoch));
        // Focus/target changes invalidate queued work and end gestures.
        invalidatePending(QStringLiteral("context changed"));
        finishAllGestures();
    }
    emitContext(false);
}

void MockKdenlive::emitContext(bool deferred)
{
    if (m_leases.isEmpty()) {
        return;  // no subscribers: no timers, no signals
    }
    if (!deferred && m_contextLagMs > 0) {
        // A busy GUI: the context follows acks and replies late.
        if (!m_contextTimer->isActive()) {
            m_contextTimer->start(m_contextLagMs);
        }
        return;
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

QVariantMap MockKdenlive::effectParamDescriptor(const QString &name, int effectIndex) const
{
    return {{kOptTarget, QStringLiteral("eff-%1-par-%2").arg(effectIndex).arg(name)},
            {QStringLiteral("name"), name},
            {QStringLiteral("type"), QStringLiteral("number")},
            {QStringLiteral("value"), m_params.value(name)},
            {QStringLiteral("min"), name == QLatin1String("saturation") ? 0.0 : -1.0},
            {QStringLiteral("max"), name == QLatin1String("saturation") ? 2.0 : 1.0},
            {QStringLiteral("step"), 0.01},
            {QStringLiteral("fineStep"), 0.001},
            {QStringLiteral("unit"), QString()},
            {QStringLiteral("frame"), -1},
            {QStringLiteral("keyframed"), false},
            {QStringLiteral("liveGrading"), true},
            {QStringLiteral("enabled"), true}};
}

QVariantMap MockKdenlive::effectDescriptor() const
{
    if (!m_effectStackShown) {
        return {};
    }
    QVariantMap e{{QStringLiteral("ownerId"), 12},
                  {QStringLiteral("sequence"), QStringLiteral("seq-1")},
                  {QStringLiteral("stack"), QVariantMap{{kOptTarget, QStringLiteral("stack-owner-1")},
                                                        {QStringLiteral("count"), m_effectStack.size()},
                                                        {QStringLiteral("enabled"), m_effectStackEnabled},
                                                        {QStringLiteral("compare"), m_effectCompare},
                                                        {QStringLiteral("compareAvailable"), m_effectCompareAvailable}}}};
    if (m_focusedEffect >= 0 && m_focusedEffect < m_effectStack.size()) {
        const QVariantMap item = m_effectStack.at(m_focusedEffect).toMap();
        e.insert(kOptTarget, QStringLiteral("eff-%1").arg(m_focusedEffect));
        e.insert(QStringLiteral("id"), item.value(QStringLiteral("id")));
        e.insert(QStringLiteral("index"), m_focusedEffect);
        e.insert(QStringLiteral("enabled"), item.value(QStringLiteral("enabled")));
        QVariantMap params;
        int n = 0;
        for (const QString &name : {QStringLiteral("rOffset"), QStringLiteral("gOffset"), QStringLiteral("bOffset"), QStringLiteral("saturation")}) {
            if (++n > 32) {
                break;
            }
            params.insert(name, effectParamDescriptor(name, m_focusedEffect));
        }
        if (!params.isEmpty()) {
            e.insert(QStringLiteral("params"), params);
        }
    }
    return e;
}

QVariantMap MockKdenlive::findParamTarget(const QString &target) const
{
    const QVariantMap p = m_context.value(QStringLiteral("param")).toMap();
    if (!target.isEmpty() && p.value(kOptTarget).toString() == target) {
        return p;
    }
    const QVariantMap params = m_context.value(QStringLiteral("effect")).toMap().value(QStringLiteral("params")).toMap();
    for (auto it = params.cbegin(); it != params.cend(); ++it) {
        const QVariantMap d = it.value().toMap();
        if (d.value(kOptTarget).toString() == target) {
            return d;
        }
    }
    return {};
}

QStringList MockKdenlive::selectedTrimClips() const
{
    QStringList clips;
    if (m_selectedClip.isEmpty() || !m_clips.contains(m_selectedClip)) {
        return clips;
    }
    clips << m_selectedClip;
    const QString linked = m_clips.value(m_selectedClip).toMap().value(QStringLiteral("linked")).toString();
    if (!linked.isEmpty() && m_clips.contains(linked)) {
        clips << linked;
    }
    clips.removeDuplicates();
    std::sort(clips.begin(), clips.end());
    return clips;
}

QStringList MockKdenlive::rippleScopeClips() const
{
    const QStringList selected = selectedTrimClips();
    if (selected.isEmpty()) {
        return {};
    }
    QStringList selectedTracks;
    const int boundary = m_clips.value(selected.first()).toMap().value(QStringLiteral("end")).toInt();
    for (const auto &id : selected) {
        const QVariantMap c = m_clips.value(id).toMap();
        selectedTracks << c.value(QStringLiteral("track")).toString();
    }
    selectedTracks.removeDuplicates();
    QStringList out;
    for (auto it = m_clips.cbegin(); it != m_clips.cend(); ++it) {
        if (selected.contains(it.key())) {
            continue;
        }
        const QVariantMap c = it.value().toMap();
        if (selectedTracks.contains(c.value(QStringLiteral("track")).toString()) && c.value(QStringLiteral("start")).toInt() >= boundary) {
            out << it.key();
        }
    }
    std::sort(out.begin(), out.end());
    if (!rippleAvailable()) {
        out.clear();
    }
    return out;
}

bool MockKdenlive::rippleAvailable() const
{
    const QStringList selected = selectedTrimClips();
    if (selected.isEmpty() || m_clips.size() > 2000) {
        return false;
    }
    QStringList selectedTracks;
    const int boundary = m_clips.value(selected.first()).toMap().value(QStringLiteral("end")).toInt();
    for (const auto &id : selected) {
        selectedTracks << m_clips.value(id).toMap().value(QStringLiteral("track")).toString();
    }
    selectedTracks.removeDuplicates();
    QStringList following;
    for (auto it = m_clips.cbegin(); it != m_clips.cend(); ++it) {
        if (selected.contains(it.key())) {
            continue;
        }
        const QVariantMap c = it.value().toMap();
        if (!selectedTracks.contains(c.value(QStringLiteral("track")).toString())) {
            continue;
        }
        if (c.value(QStringLiteral("end")).toInt() > boundary && c.value(QStringLiteral("start")).toInt() < boundary) {
            return false;
        }
        if (c.value(QStringLiteral("start")).toInt() >= boundary) {
            following << it.key();
        }
    }
    if (following.size() > 64) {
        return false;
    }
    for (const auto &id : following) {
        const QString linked = m_clips.value(id).toMap().value(QStringLiteral("linked")).toString();
        if (!linked.isEmpty() && !following.contains(linked) && !selected.contains(linked)) {
            return false;
        }
    }
    return true;
}

QStringList MockKdenlive::nudgeClips() const
{
    QStringList clips = m_nudgeSelection;
    if (clips.isEmpty()) {
        clips = selectedTrimClips();
    }
    clips.removeDuplicates();
    std::sort(clips.begin(), clips.end());
    return clips;
}

QStringList MockKdenlive::visibleBinClipIds() const
{
    if (!m_binVisible) {
        return {};
    }
    QStringList ids;
    for (const auto &v : m_binClips) {
        const QVariantMap c = v.toMap();
        const QStringList tags = c.value(QStringLiteral("tags")).toStringList();
        bool visible = true;
        for (const auto &tag : m_binFilterTags) {
            visible = visible && tags.contains(tag);
        }
        if (!m_binFilterRatings.isEmpty()) {
            const int nativeRating = c.value(QStringLiteral("rating")).toInt();
            visible = visible && nativeRating % 2 == 0 && m_binFilterRatings.contains(nativeRating / 2);
        }
        if (visible) {
            ids << c.value(QStringLiteral("id")).toString();
        }
    }
    return ids;
}

QStringList MockKdenlive::binTagIds() const
{
    return {QStringLiteral("#ff0000"), QStringLiteral("#00ff00"), QStringLiteral("#0000ff"), QStringLiteral("#ffff00"), QStringLiteral("#00ffff")};
}

bool MockKdenlive::validBinSelection() const
{
    if (!m_binVisible || m_binSelection.isEmpty() || m_binSelection.size() > 256) {
        return false;
    }
    for (const QString &id : m_binSelection) {
        bool found = false;
        for (const auto &v : m_binClips) {
            const QVariantMap c = v.toMap();
            if (c.value(QStringLiteral("id")).toString() == id && c.value(QStringLiteral("complete"), true).toBool()) {
                found = true;
                break;
            }
        }
        if (!found) {
            return false;
        }
    }
    return true;
}

QString MockKdenlive::binSelectionTarget() const
{
    return validBinSelection() ? QStringLiteral("bin-sel-") + m_binSelection.join(QLatin1Char('-')) : QString();
}

QVariantMap MockKdenlive::binDescriptor() const
{
    if (!m_binVisible) {
        return {};
    }
    QVariantMap selection{{QStringLiteral("count"), m_binSelection.size()}, {QStringLiteral("available"), validBinSelection()}};
    if (validBinSelection()) {
        selection.insert(kOptTarget, binSelectionTarget());
        QStringList tags = binTagIds();
        bool first = true;
        int rating = -1;
        bool mixedRating = false;
        for (const auto &id : m_binSelection) {
            for (const auto &v : m_binClips) {
                const QVariantMap c = v.toMap();
                if (c.value(QStringLiteral("id")).toString() != id) {
                    continue;
                }
                const QStringList clipTags = c.value(QStringLiteral("tags")).toStringList();
                if (first) {
                    tags = clipTags;
                    rating = c.value(QStringLiteral("rating")).toInt();
                    first = false;
                } else {
                    tags.erase(std::remove_if(tags.begin(), tags.end(), [&](const QString &t) { return !clipTags.contains(t); }), tags.end());
                    mixedRating = mixedRating || rating != c.value(QStringLiteral("rating")).toInt();
                }
            }
        }
        selection.insert(QStringLiteral("tags"), tags);
        // Kdenlive's QVariant() "null" aborts libdbus (reported to MAIN); a typed
        // null double would read as 0 stars. Mixed ratings omit the key instead.
        if (!mixedRating) {
            selection.insert(QStringLiteral("rating"), double(rating) / 2.0);
        }
        selection.insert(QStringLiteral("mixedRating"), mixedRating);
    } else if (!m_binSelection.isEmpty()) {
        selection.insert(QStringLiteral("reason"), QStringLiteral("Only selections of1..256 complete bin clips support these metadata controls."));
    }
    QVariantList tags;
    const QStringList names{QStringLiteral("Red"), QStringLiteral("Green"), QStringLiteral("Blue"), QStringLiteral("Yellow"), QStringLiteral("Cyan")};
    const QStringList ids = binTagIds();
    for (int i = 0; i < ids.size(); ++i) {
        tags << QVariantMap{{QStringLiteral("id"), ids.at(i)}, {QStringLiteral("name"), names.at(i)}};
    }
    return {{QStringLiteral("selection"), selection},
            {QStringLiteral("tags"), tags},
            {QStringLiteral("filter"), QVariantMap{{QStringLiteral("tags"), m_binFilterTags},
                                                   {QStringLiteral("ratings"), m_binFilterRatings},
                                                   {QStringLiteral("types"), m_binFilterTypes},
                                                   {QStringLiteral("usage"), m_binFilterUsage}}}};
}

int MockKdenlive::effectIndexForTarget(const QString &target) const
{
    if (!target.startsWith(QLatin1String("eff-"))) {
        return -1;
    }
    bool ok = false;
    const int idx = target.mid(4).toInt(&ok);
    return ok && idx >= 0 && idx < m_effectStack.size() ? idx : -1;
}

void MockKdenlive::refreshDescriptors()
{
    m_context.insert(QStringLiteral("timeline"), timelineDescriptor());
    if (m_effectStackShown) {
        if (m_stage >= 4) {
            m_context.insert(QStringLiteral("effect"), effectDescriptor());
        } else if (m_context.contains(QStringLiteral("colorWheel"))) {
            m_context.insert(QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("lift_gamma_gain")},
                                                                   {QStringLiteral("ownerId"), 12},
                                                                   {QStringLiteral("sequence"), QStringLiteral("seq-1")}});
        } else if (m_context.contains(QStringLiteral("param"))) {
            m_context.insert(QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("brightness")},
                                                                   {QStringLiteral("ownerId"), 12},
                                                                   {QStringLiteral("sequence"), QStringLiteral("seq-1")}});
        }
    } else if (!m_context.contains(QStringLiteral("colorWheel")) && !m_context.contains(QStringLiteral("param"))) {
        m_context.remove(QStringLiteral("effect"));
    }
    if (m_binVisible) {
        m_context.insert(QStringLiteral("bin"), binDescriptor());
    } else {
        m_context.remove(QStringLiteral("bin"));
    }
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
        m_effectStackShown = false;
        m_context.remove(QStringLiteral("effect"));
        m_context.insert(QStringLiteral("focus"), QStringLiteral("timeline"));
    } else {
        m_context.remove(QStringLiteral("param"));
        m_effectStackShown = true;
        m_effectStack = {effect(QStringLiteral("lift_gamma_gain"))};
        m_focusedEffect = 0;
        m_focusedEffectParam = QStringLiteral("rOffset");
        m_context.insert(QStringLiteral("focus"), QStringLiteral("effectStack"));
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
        m_effectStackShown = false;
        m_context.remove(QStringLiteral("effect"));
        m_context.insert(QStringLiteral("focus"), QStringLiteral("timeline"));
    } else {
        m_context.remove(QStringLiteral("colorWheel"));
        m_context.remove(QStringLiteral("colorWheels"));
        m_effectStackShown = true;
        m_effectStack = {effect(QStringLiteral("brightness"))};
        m_focusedEffect = 0;
        m_focusedEffectParam = name;
        m_context.insert(QStringLiteral("focus"), QStringLiteral("effectStack"));
        m_context.insert(QStringLiteral("param"), paramDescriptor(name));
        refreshDescriptors();
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
            {QStringLiteral("targets"), QVariantMap{{QStringLiteral("video"), m_videoTarget}, {QStringLiteral("audio"), m_audioTargets}}},
            {QStringLiteral("binClips"), m_binClips},
            {QStringLiteral("binSelection"), m_binSelection},
            {QStringLiteral("binCursor"), m_binCursor},
            {QStringLiteral("effectStack"), m_effectStack},
            {QStringLiteral("focusedEffect"), m_focusedEffect},
            {QStringLiteral("triggered"), m_triggered},
            {QStringLiteral("history"), m_history},
            {QStringLiteral("redo"), m_redo},
            {QStringLiteral("zone"), QVariantMap{{QStringLiteral("in"), m_zoneIn}, {QStringLiteral("out"), m_zoneOut}}},
            {QStringLiteral("editMode"), m_editMode},
            {QStringLiteral("sourceOpen"), m_sourceOpen},
            {QStringLiteral("trimmingPreview"), m_trimming},
            {QStringLiteral("epoch"), u64(m_epoch)}};
}

} // namespace cs
