// SPDX-License-Identifier: GPL-2.0-or-later
#include "cheatsheet.h"
#include "bindinglabel.h"
#include "installed.h"
#include "keysink.h"

#include <QJsonArray>
#include <algorithm>
#include <QJsonDocument>
#include <QSet>
#include <QTimer>

namespace cs {

Cheatsheet::Cheatsheet(Engine *engine, KdenliveClient *kd, QObject *parent)
    : QObject(parent)
    , m_engine(engine)
    , m_kd(kd)
    , m_layout([] { return *builtinBoardProfile(QStringLiteral("sy181-15k3e")); })
    , m_debounce(new QTimer(this))
    , m_autoHide(new QTimer(this))
{
    // Kdenlive's context ticks with the playhead; coalesce before rebuilding.
    m_debounce->setSingleShot(true);
    m_debounce->setInterval(40);
    connect(m_debounce, &QTimer::timeout, this, &Cheatsheet::refresh);
    m_autoHide->setSingleShot(true);
    connect(m_autoHide, &QTimer::timeout, this, &Cheatsheet::hide);
    connect(m_engine, &Engine::resolutionChanged, this, &Cheatsheet::invalidate);
    connect(m_engine, &Engine::cheatsheetRequested, this, &Cheatsheet::request);
    if (m_kd) {
        connect(m_kd, &KdenliveClient::contextChanged, this, &Cheatsheet::invalidate);
        connect(m_kd, &KdenliveClient::stateChanged, this, &Cheatsheet::invalidate);
    }
}

void Cheatsheet::setLayoutProvider(std::function<BoardProfile()> f)
{
    m_layout = std::move(f);
    invalidate();
}

void Cheatsheet::invalidate()
{
    if (m_visible && !m_debounce->isActive()) {
        m_debounce->start();
    }
}

void Cheatsheet::refresh()
{
    if (!m_visible) {
        return;
    }
    const QJsonObject c = content();
    const QByteArray bytes = QJsonDocument(c).toJson(QJsonDocument::Compact);
    if (bytes != m_last) {
        m_last = bytes;
        Q_EMIT changed(c);
    }
}

void Cheatsheet::show(bool byHold)
{
    const int autoHide = m_engine->config().cheatsheet.effectiveAutoHideMs();
    if (!byHold && autoHide > 0) {
        m_autoHide->start(autoHide);
    } else {
        m_autoHide->stop();  // a held key hides it on release
    }
    if (m_visible) {
        return;
    }
    m_visible = true;
    const QJsonObject c = content();
    m_last = QJsonDocument(c).toJson(QJsonDocument::Compact);
    Q_EMIT visibilityChanged(true);
    Q_EMIT changed(c);
}

void Cheatsheet::hide()
{
    m_autoHide->stop();
    m_debounce->stop();
    if (!m_visible) {
        return;
    }
    m_visible = false;
    Q_EMIT visibilityChanged(false);
}

void Cheatsheet::forceHide()
{
    hide();
    Q_EMIT hideForced();
}

bool Cheatsheet::autoHideActive() const
{
    return m_autoHide->isActive();
}

int Cheatsheet::autoHideIntervalMs() const
{
    return m_autoHide->interval();
}

void Cheatsheet::toggle()
{
    if (m_visible) {
        hide();
    } else {
        show(false);
    }
}

void Cheatsheet::request(const QString &op)
{
    if (op == QLatin1String("show")) {
        show(true);
    } else if (op == QLatin1String("hide")) {
        hide();
    } else {
        toggle();
    }
}

void Cheatsheet::noteInput()
{
    if (m_visible && m_autoHide->isActive()) {
        m_autoHide->start();  // same interval, from now
    }
}

bool Cheatsheet::isActive(const Binding &b) const
{
    const Profile *p = m_engine->activeProfile();
    const bool kdProfile = p && p->kdenlive;
    const bool viaKeys = !kdProfile || m_engine->kdenliveStock();  // stock shortcuts are typed
    switch (b.kind) {
    case Binding::Action:
        if (viaKeys) {
            return !b.keys.isEmpty();
        }
        return m_engine->kdenliveActive() && m_kd->supportsAction(b.name) && m_kd->actionEnabled(b.name);
    case Binding::Control: {
        if (viaKeys) {
            return !b.keys.isEmpty();
        }
        const QString name = b.name.startsWith(QLatin1Char('$')) ? m_engine->modeValue(b.name.mid(1)) : b.name;
        return m_engine->kdenliveActive() && m_kd->supportsControl(name);
    }
    case Binding::Request:
        return m_engine->kdenliveActive() && m_kd->supportsCommand(b.name);
    case Binding::Command:
        return isInstalled(b.argv.first());  // greyed out when the program is missing
    case Binding::Sequence:
        // Greyed out when any step cannot run (e.g. an action Kdenlive does not offer yet).
        return std::all_of(b.steps.cbegin(), b.steps.cend(), [this](const Binding &s) { return isActive(s); });
    case Binding::None:
        return false;
    default:
        return true;
    }
}

QJsonObject Cheatsheet::entry(const std::optional<Engine::Resolution> &r) const
{
    if (!r || !r->binding.isValid()) {
        return QJsonObject{{QStringLiteral("bound"), false},       {QStringLiteral("label"), QString()},
                           {QStringLiteral("kind"), QStringLiteral("none")}, {QStringLiteral("custom"), false},
                           {QStringLiteral("state"), QString()},   {QStringLiteral("binding"), QString()},
                           {QStringLiteral("profile"), r ? r->profile : QString()}, {QStringLiteral("layer"), r ? r->layer : QString()},
                           {QStringLiteral("active"), false}, {QStringLiteral("icon"), QString()}};
    }
    const Binding &b = r->binding;
    LabelEnv env;
    env.modeValue = [this](const QString &m) { return m_engine->modeValue(m); };
    env.context = m_engine->kdenliveActive() ? m_kd->context() : QVariantMap{};
    return QJsonObject{{QStringLiteral("bound"), true},
                       {QStringLiteral("label"), bindingLabel(b, env)},
                       {QStringLiteral("kind"), bindingKindName(b.kind)},
                       {QStringLiteral("custom"), !b.label.isEmpty()},
                       {QStringLiteral("state"), bindingState(b, env)},
                       {QStringLiteral("binding"), b.describe()},
                       {QStringLiteral("profile"), r->profile},
                       {QStringLiteral("layer"), r->layer},
                       {QStringLiteral("active"), isActive(b)},
                       {QStringLiteral("icon"), bindingIcon(b, env)}};
}

QJsonObject Cheatsheet::content() const
{
    const BoardProfile layout = m_layout();
    QStringList layers;
    auto note = [&layers](const QJsonObject &e) {
        const QString l = e.value(QStringLiteral("layer")).toString();
        if (e.value(QStringLiteral("bound")).toBool() && !l.isEmpty() && !layers.contains(l)) {
            layers << l;
        }
        return e;
    };
    QJsonArray keys, knobs;
    for (const BoardKey &k : layout.keys) {
        QJsonObject o = note(entry(m_engine->resolve(k.control)));
        o.insert(QStringLiteral("control"), k.control);
        o.insert(QStringLiteral("row"), k.row);
        o.insert(QStringLiteral("column"), k.column);
        keys.append(o);
    }
    for (const BoardKnob &k : layout.knobs) {
        knobs.append(QJsonObject{{QStringLiteral("control"), k.control},
                                 {QStringLiteral("row"), k.row},
                                 {QStringLiteral("column"), k.column},
                                 {QStringLiteral("ccw"), note(entry(m_engine->resolve(Engine::turnSlots(k.control, -1))))},
                                 {QStringLiteral("press"), note(entry(m_engine->resolve(k.control + QStringLiteral(".press"))))},
                                 {QStringLiteral("cw"), note(entry(m_engine->resolve(Engine::turnSlots(k.control, 1))))},
                                 {QStringLiteral("shiftCcw"), note(entry(m_engine->resolve(Engine::shiftSlots(k.control, -1))))},
                                 {QStringLiteral("shiftCw"), note(entry(m_engine->resolve(Engine::shiftSlots(k.control, 1))))}});
    }
    const Profile *p = m_engine->activeProfile();
    const QString profile = p ? p->name : QString();
    QString notice;
    if (p && p->kdenlive) {
        if (m_engine->kdenliveAbsent() && !m_engine->kdenliveStock()) {
            notice = Engine::absentNotice();
        } else if (!m_engine->kdenliveActive() && !m_engine->kdenliveStock()) {
            notice = QStringLiteral("Waiting for Kdenlive to answer");
        }
    }
    const CheatsheetOptions &o = m_engine->config().cheatsheet;
    const WindowInfo &w = m_engine->activeWindow();
    QJsonObject out{{QStringLiteral("ok"), true},
                    {QStringLiteral("visible"), m_visible},
                    {QStringLiteral("title"), layers.isEmpty() ? profile : QStringLiteral("%1 · %2").arg(profile, layers.join(QStringLiteral(", ")))},
                    {QStringLiteral("profile"), profile},
                    {QStringLiteral("layers"), QJsonArray::fromStringList(layers)},
                    // Held-layer keys down now ("when": {"held": ...}), e.g. ["key1"].
                    {QStringLiteral("held"), QJsonArray::fromStringList(m_engine->heldModifiers())},
                    {QStringLiteral("window"), QJsonObject{{QStringLiteral("class"), w.cls}, {QStringLiteral("title"), w.title}}},
                    {QStringLiteral("notice"), notice},
                    {QStringLiteral("options"), QJsonObject{{QStringLiteral("opacity"), o.opacity},
                                                            {QStringLiteral("autoHideMs"), o.effectiveAutoHideMs()},
                                                            {QStringLiteral("position"), o.position}}},
                    {QStringLiteral("layout"), QJsonObject{{QStringLiteral("id"), layout.id},
                                                           {QStringLiteral("name"), layout.name},
                                                           {QStringLiteral("rows"), layout.rows},
                                                           {QStringLiteral("columns"), layout.columns},
                                                           {QStringLiteral("source"), layout.source}}},
                    {QStringLiteral("keys"), keys},
                    {QStringLiteral("knobs"), knobs}};
    if (m_engine->kdenliveActive()) {
        // Only what layers usually test; the playhead is left out so the
        // content does not change with every frame.
        out.insert(QStringLiteral("context"), QJsonObject{{QStringLiteral("focus"), m_kd->context().value(QStringLiteral("focus")).toString()}});
    }
    return out;
}

QJsonObject Cheatsheet::preview(const Config &cfg, const BoardProfile &layout, const QString &windowClass, const QString &title,
                                const QVariantMap &kdenliveContext)
{
    RecordingKeySink keys;
    FakeKdenliveClient kd;
    // A preview shows what the bindings do with Kdenlive's interface on.
    kd.setState(KdenliveClient::State::Available);
    QVariantMap ctx = kdenliveContext;
    QStringList held;
    const QVariant heldValue = ctx.take(QStringLiteral("$held"));
    for (const QString &h : heldValue.typeId() == QMetaType::QString ? QStringList{heldValue.toString()} : heldValue.toStringList()) {
        for (const QString &c : h.split(QLatin1Char('+'), Qt::SkipEmptyParts)) {
            held << c.trimmed();
        }
    }
    kd.setContext(ctx);
    Engine engine(&keys, &kd);
    engine.setConfig(cfg);
    engine.setActiveWindow(WindowInfo{windowClass, title, 4242, QString()});
    engine.setHeldForPreview(held);
    Cheatsheet sheet(&engine, &kd);
    sheet.setLayoutProvider([layout] { return layout; });
    QJsonObject c = sheet.content();
    c.insert(QStringLiteral("preview"), true);
    return c;
}

QJsonObject Cheatsheet::previewFor(const QString &windowClass, const QString &title, const QVariantMap &kdenliveContext) const
{
    return preview(m_engine->config(), m_layout(), windowClass, title, kdenliveContext);
}

} // namespace cs
