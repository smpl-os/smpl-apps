// SPDX-License-Identifier: GPL-2.0-or-later
#include "settingsservice.h"
#include "featurelist.h"
#include "kdenlivecatalog.h"

#include <QCryptographicHash>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusServiceWatcher>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>
#include <QTimer>

namespace cs {

QJsonObject DeviceState::toJson() const
{
    if (!present) {
        return QJsonObject{{QStringLiteral("present"), false}};
    }
    return QJsonObject{{QStringLiteral("present"), true},
                       {QStringLiteral("vendor"), usb.vendor},
                       {QStringLiteral("product"), usb.product},
                       {QStringLiteral("serial"), usb.serial},
                       {QStringLiteral("manufacturer"), usb.manufacturer},
                       {QStringLiteral("productName"), usb.productName},
                       {QStringLiteral("bcdDevice"), usb.bcdDevice},
                       {QStringLiteral("firmware"), firmware.toJson()},
                       {QStringLiteral("inputMode"), inputMode},
                       {QStringLiteral("devnodes"), QJsonArray::fromStringList(devnodes)}};
}

QJsonObject PluginInfo::toJson() const
{
    QJsonObject o{{QStringLiteral("id"), id},
                  {QStringLiteral("name"), name},
                  {QStringLiteral("tier"), tier},
                  {QStringLiteral("status"), status},
                  {QStringLiteral("detail"), detail},
                  {QStringLiteral("apps"), QJsonArray::fromStringList(apps)}};
    for (auto it = extra.begin(); it != extra.end(); ++it) {
        o.insert(it.key(), it.value());
    }
    return o;
}

SettingsService::SettingsService(const QString &configPath, QObject *parent)
    : QObject(parent)
    , m_store(configPath)
    , m_bus(QStringLiteral("cs-settings-unset"))
    , m_identifyTimer(new QTimer(this))
    , m_ownerWatch(new QDBusServiceWatcher(this))
{
    m_identifyTimer->setSingleShot(true);
    connect(m_identifyTimer, &QTimer::timeout, this, &SettingsService::endIdentify);
    m_ownerWatch->setWatchMode(QDBusServiceWatcher::WatchForUnregistration);
    connect(m_ownerWatch, &QDBusServiceWatcher::serviceUnregistered, this, [this](const QString &name) {
        if (name == m_identifyOwner) {
            endIdentify();  // the Settings app went away: never leave the pad muted
        }
    });
}

bool SettingsService::registerOn(const QDBusConnection &bus, bool claimName, QString *error)
{
    m_bus = bus;
    if (!m_bus.registerObject(QLatin1String(kPath), this,
                              QDBusConnection::ExportScriptableSlots | QDBusConnection::ExportScriptableSignals | QDBusConnection::ExportAllProperties)) {
        if (error) {
            *error = QStringLiteral("cannot export %1: %2").arg(QLatin1String(kPath), m_bus.lastError().message());
        }
        return false;
    }
    if (claimName && !m_bus.registerService(QLatin1String(kService))) {
        m_bus.unregisterObject(QLatin1String(kPath));
        if (error) {
            *error = QStringLiteral("cannot own %1 (another daemon or the mock running?): %2").arg(QLatin1String(kService), m_bus.lastError().message());
        }
        return false;
    }
    m_ownerWatch->setConnection(m_bus);
    m_registered = true;
    return true;
}

QString SettingsService::json(const QJsonObject &o)
{
    return QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact));
}

void SettingsService::propertiesChanged(const QVariantMap &changed)
{
    if (!m_registered || changed.isEmpty()) {
        return;
    }
    QDBusMessage sig = QDBusMessage::createSignal(QLatin1String(kPath), QStringLiteral("org.freedesktop.DBus.Properties"), QStringLiteral("PropertiesChanged"));
    sig << QLatin1String(kInterface) << changed << QStringList();
    m_bus.send(sig);
}

void SettingsService::setMode(const QString &mode)
{
    if (mode != m_mode) {
        m_mode = mode;
        propertiesChanged({{QStringLiteral("Mode"), mode}});
    }
}

void SettingsService::setDevice(const DeviceState &d)
{
    const bool presenceChanged = d.present != m_device.present;
    const QString oldType = firmwareType();
    m_device = d;
    QVariantMap changed;
    if (presenceChanged) {
        changed.insert(QStringLiteral("DevicePresent"), d.present);
    }
    if (firmwareType() != oldType) {
        changed.insert(QStringLiteral("FirmwareType"), firmwareType());
    }
    propertiesChanged(changed);
    if (!changed.isEmpty()) {
        Q_EMIT DeviceChanged(d.present, firmwareType());
    }
}

void SettingsService::setActiveProfile(const QString &name)
{
    if (name != m_profile) {
        m_profile = name;
        propertiesChanged({{QStringLiteral("ActiveProfile"), name}});
    }
}

void SettingsService::setActiveLayer(const QString &name)
{
    if (name != m_layer) {
        m_layer = name;
        propertiesChanged({{QStringLiteral("ActiveLayer"), name}});
    }
}

void SettingsService::setActiveWindow(const QString &cls, const QString &title)
{
    m_windowClass = cls;
    m_windowTitle = title;
}

void SettingsService::setPlugins(const QList<PluginInfo> &plugins)
{
    QJsonArray before, after;
    for (const PluginInfo &p : std::as_const(m_plugins)) {
        before.append(p.toJson());
    }
    for (const PluginInfo &p : plugins) {
        after.append(p.toJson());
    }
    m_plugins = plugins;
    if (before != after) {
        Q_EMIT PluginsChanged();
    }
}

void SettingsService::setConfigState(const QString &hash, const QString &error, const QStringList &warnings)
{
    const bool changed = hash != m_configHash;
    m_configError = error;
    if (error.isEmpty()) {
        m_configWarnings = warnings;
    }
    if (changed && error.isEmpty()) {
        m_configHash = hash;
        propertiesChanged({{QStringLiteral("ConfigHash"), hash}});
        Q_EMIT ConfigChanged(hash);
    } else if (!error.isEmpty()) {
        Q_EMIT ConfigRejected(error);
    }
}

// ---------------------------------------------------------------------------------
// Identify
// ---------------------------------------------------------------------------------

bool SettingsService::identifyActive() const
{
    return m_identifyTimer->isActive();
}

void SettingsService::SetIdentify(bool on)
{
    const bool was = identifyActive();
    if (!on) {
        if (was) {
            endIdentify();
        }
        return;
    }
    const QString owner = calledFromDBus() ? message().service() : QString();
    if (owner != m_identifyOwner) {
        if (!m_identifyOwner.isEmpty()) {
            m_ownerWatch->removeWatchedService(m_identifyOwner);
        }
        m_identifyOwner = owner;
        if (!owner.isEmpty()) {
            m_ownerWatch->addWatchedService(owner);
        }
    }
    m_identifyTimer->start(kIdentifySafetyMs);  // calling again renews
    if (!was) {
        propertiesChanged({{QStringLiteral("Identifying"), true}});
        Q_EMIT IdentifyChanged(true);
    }
}

void SettingsService::endIdentify()
{
    m_identifyTimer->stop();
    if (!m_identifyOwner.isEmpty()) {
        m_ownerWatch->removeWatchedService(m_identifyOwner);
        m_identifyOwner.clear();
    }
    propertiesChanged({{QStringLiteral("Identifying"), false}});
    Q_EMIT IdentifyChanged(false);
}

void SettingsService::inputEventFor(const PadEvent &e, QString *slot, QString *event, int *delta)
{
    *slot = e.control;
    *delta = 0;
    switch (e.type) {
    case PadEvent::KeyDown:
    case PadEvent::PressDown:
        *event = QStringLiteral("press");
        break;
    case PadEvent::KeyUp:
    case PadEvent::PressUp:
        *event = QStringLiteral("release");
        break;
    case PadEvent::Turn:
        *event = e.delta < 0 ? QStringLiteral("ccw") : QStringLiteral("cw");
        *delta = e.delta;
        break;
    }
}

bool SettingsService::filterPadEvent(const PadEvent &e)
{
    QString slot, event;
    int delta = 0;
    inputEventFor(e, &slot, &event, &delta);
    Q_EMIT InputEvent(slot, event, delta);
    return identifyActive();
}

// ---------------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------------

QJsonObject SettingsService::layoutJson() const
{
    if (m_device.present && !m_device.firmware.board.isEmpty()) {
        if (auto p = builtinBoardProfile(m_device.firmware.board)) {
            QJsonObject o = p->toJson();
            o.insert(QStringLiteral("source"), QStringLiteral("firmware"));
            return o;
        }
    }
    if (m_fallbackLayout) {
        return m_fallbackLayout->toJson();
    }
    return {};
}

QString SettingsService::GetStatus()
{
    QJsonObject identify{{QStringLiteral("active"), identifyActive()},
                         {QStringLiteral("remainingMs"), identifyActive() ? m_identifyTimer->remainingTime() : 0}};
    return json(QJsonObject{{QStringLiteral("ok"), true},
                            {QStringLiteral("apiVersion"), int(kApiVersion)},
                            {QStringLiteral("daemonVersion"), m_version},
                            {QStringLiteral("mode"), m_mode},
                            {QStringLiteral("device"), m_device.toJson()},
                            {QStringLiteral("activeProfile"), m_profile},
                            {QStringLiteral("activeLayer"), m_layer},
                            {QStringLiteral("window"), QJsonObject{{QStringLiteral("class"), m_windowClass}, {QStringLiteral("title"), m_windowTitle}}},
                            {QStringLiteral("config"), QJsonObject{{QStringLiteral("path"), m_store.path()},
                                                                   {QStringLiteral("hash"), m_configHash},
                                                                   {QStringLiteral("error"), m_configError},
                                                                   {QStringLiteral("warnings"), QJsonArray::fromStringList(m_configWarnings)}}},
                            {QStringLiteral("identify"), identify},
                            {QStringLiteral("layout"), layoutJson()},
                            {QStringLiteral("flash"), m_job ? QJsonValue(m_job->toJson()) : QJsonValue()}});
}

QString SettingsService::GetDevice()
{
    QJsonObject o = m_device.toJson();
    o.insert(QStringLiteral("ok"), true);
    o.insert(QStringLiteral("layout"), layoutJson());
    return json(o);
}

QString SettingsService::GetLayout()
{
    const QJsonObject l = layoutJson();
    if (l.isEmpty()) {
        return json(QJsonObject{{QStringLiteral("ok"), false}, {QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), QStringLiteral("unknown")}, {QStringLiteral("message"), QStringLiteral("no layout known for this pad")}}}});
    }
    return json(QJsonObject{{QStringLiteral("ok"), true}, {QStringLiteral("layout"), l}});
}

QString SettingsService::ListBoardProfiles()
{
    QJsonArray a;
    for (const BoardProfile &p : builtinBoardProfiles()) {
        a.append(p.toJson());
    }
    return json(QJsonObject{{QStringLiteral("ok"), true}, {QStringLiteral("profiles"), a}});
}

QString SettingsService::ListPlugins()
{
    QJsonArray a;
    for (const PluginInfo &p : std::as_const(m_plugins)) {
        a.append(p.toJson());
    }
    return json(QJsonObject{{QStringLiteral("ok"), true}, {QStringLiteral("plugins"), a}});
}

QString SettingsService::GetFeatures()
{
    QJsonObject o = featuresJson();
    o.insert(QStringLiteral("ok"), true);
    return json(o);
}

QString SettingsService::GetCatalog(const QString &pluginId)
{
    if (pluginId == QLatin1String("kdenlive")) {
        QJsonObject o = catalog::toJson();
        o.insert(QStringLiteral("ok"), true);
        return json(o);
    }
    return json(QJsonObject{{QStringLiteral("ok"), false},
                            {QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), QStringLiteral("unknown-plugin")}, {QStringLiteral("message"), QStringLiteral("no catalog for %1").arg(pluginId)}}}});
}

// ---------------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------------

QString SettingsService::GetConfig(QString &path, QString &hash)
{
    const auto s = m_store.read();
    path = m_store.path();
    hash = s.hash;
    return QString::fromUtf8(s.text);
}

QString SettingsService::ValidateConfig(const QString &text)
{
    return json(m_store.validate(text.toUtf8()).toJson());
}

QString SettingsService::applyText(const QByteArray &text, const QString &hash, QJsonObject *out)
{
    const auto v = m_store.validate(text);
    if (!v.ok) {
        return v.errors.join(QStringLiteral("; "));
    }
    const QString err = m_apply ? m_apply(*v.config) : QString();
    if (!err.isEmpty()) {
        return err;
    }
    if (out) {
        out->insert(QStringLiteral("warnings"), QJsonArray::fromStringList(v.warnings));
    }
    setConfigState(hash, QString(), v.warnings);
    return {};
}

QString SettingsService::SetConfig(const QString &text, const QString &expectedHash)
{
    const QByteArray bytes = text.toUtf8();
    const auto r = m_store.write(bytes, expectedHash);
    QJsonObject o = r.toJson();
    if (!r.ok) {
        return json(o);
    }
    const QString err = applyText(bytes, r.hash, &o);
    if (!err.isEmpty()) {
        // Written and valid, but the running daemon could not take it.
        o.insert(QStringLiteral("ok"), false);
        o.insert(QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), QStringLiteral("apply")}, {QStringLiteral("message"), err}});
        m_configError = err;
        Q_EMIT ConfigRejected(err);
    }
    return json(o);
}

QString SettingsService::ReloadConfig()
{
    const auto s = m_store.read();
    if (!s.exists) {
        const QString err = QStringLiteral("%1 does not exist").arg(m_store.path());
        m_configError = err;
        Q_EMIT ConfigRejected(err);
        return json(QJsonObject{{QStringLiteral("ok"), false}, {QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), QStringLiteral("missing")}, {QStringLiteral("message"), err}}}});
    }
    QJsonObject o{{QStringLiteral("hash"), s.hash}};
    const QString err = applyText(s.text, s.hash, &o);
    if (!err.isEmpty()) {
        m_configError = err;
        Q_EMIT ConfigRejected(err);
        o.insert(QStringLiteral("ok"), false);
        o.insert(QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), QStringLiteral("invalid")}, {QStringLiteral("message"), err}});
        return json(o);
    }
    o.insert(QStringLiteral("ok"), true);
    return json(o);
}

// ---------------------------------------------------------------------------------
// Firmware
// ---------------------------------------------------------------------------------

QString SettingsService::GetFirmwareStatus()
{
    QJsonArray images;
    for (const QString &dir : std::as_const(m_flash.imageDirs)) {
        const QFileInfoList files = QDir(dir).entryInfoList({QStringLiteral("*.bin")}, QDir::Files, QDir::Name);
        for (const QFileInfo &fi : files) {
            if (fi.size() <= 0 || fi.size() > m_flash.maxImageBytes) {
                continue;
            }
            QFile f(fi.absoluteFilePath());
            if (!f.open(QIODevice::ReadOnly)) {
                continue;
            }
            const QString sha = QString::fromLatin1(QCryptographicHash::hash(f.readAll(), QCryptographicHash::Sha256).toHex());
            QJsonObject img{{QStringLiteral("path"), fi.absoluteFilePath()}, {QStringLiteral("size"), fi.size()}, {QStringLiteral("sha256"), sha}};
            // firmware/release/<name>.json next to the image: name, version, board, licence ...
            QFile meta(fi.absolutePath() + QLatin1Char('/') + fi.completeBaseName() + QStringLiteral(".json"));
            if (meta.size() < 65536 && meta.open(QIODevice::ReadOnly)) {
                const QJsonObject m = QJsonDocument::fromJson(meta.readAll()).object();
                img.insert(QStringLiteral("meta"), m);
                img.insert(QStringLiteral("metaMatches"), m.value(QStringLiteral("sha256")).toString() == sha);
            }
            images.append(img);
        }
    }
    const QFileInfo tool(m_flash.tool);
    return json(QJsonObject{{QStringLiteral("ok"), true},
                            {QStringLiteral("device"), m_device.toJson()},
                            {QStringLiteral("images"), images},
                            {QStringLiteral("imageDirs"), QJsonArray::fromStringList(m_flash.imageDirs)},
                            {QStringLiteral("flash"), QJsonObject{{QStringLiteral("allowed"), m_flash.allowed},
                                                                  {QStringLiteral("tool"), m_flash.tool},
                                                                  {QStringLiteral("toolFound"), tool.isFile() && tool.isExecutable()},
                                                                  {QStringLiteral("job"), m_job ? QJsonValue(m_job->toJson()) : QJsonValue()}}}});
}

QString SettingsService::StartFlash(const QString &image, const QString &sha256, bool dryRun)
{
    auto refuse = [](const QString &code, const QString &msg) {
        return json(QJsonObject{{QStringLiteral("ok"), false}, {QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), code}, {QStringLiteral("message"), msg}}}});
    };
    if (m_job && !m_job->isFinished()) {
        return refuse(QStringLiteral("busy"), QStringLiteral("flash job %1 is still running").arg(m_job->id()));
    }
    static const QRegularExpression hex(QStringLiteral("^[0-9a-fA-F]{64}$"));
    if (!hex.match(sha256).hasMatch()) {
        return refuse(QStringLiteral("invalid-arguments"), QStringLiteral("sha256 must be 64 hex digits"));
    }
    if (!dryRun && !m_flash.allowed) {
        return refuse(QStringLiteral("not-allowed"), QStringLiteral("real flashing is disabled (start the daemon with --allow-flash)"));
    }
    if (m_job) {
        m_job->deleteLater();
    }
    const QString id = QStringLiteral("flash-%1").arg(++m_jobCounter);
    auto *job = new FlashJob(id, m_flash, image, sha256, dryRun, this);
    m_job = job;
    connect(job, &FlashJob::progress, this, &SettingsService::FlashProgress);
    connect(job, &FlashJob::releaseDevice, this, &SettingsService::releaseDeviceRequested);
    connect(job, &FlashJob::reacquireDevice, this, &SettingsService::reacquireDeviceRequested);
    if (m_bootloaderRequest && m_device.present && m_device.firmware.type == QLatin1String("control-surface")) {
        job->setBootloaderRequest(m_bootloaderRequest);
    }
    if (m_jobSetup) {
        m_jobSetup(job);
    }
    // Start after the reply, so the caller has the job id before any progress.
    QTimer::singleShot(0, job, &FlashJob::start);
    return json(QJsonObject{{QStringLiteral("ok"), true}, {QStringLiteral("jobId"), id}, {QStringLiteral("dryRun"), dryRun}});
}

bool SettingsService::CancelFlash(const QString &jobId)
{
    return m_job && m_job->id() == jobId && m_job->cancel();
}

} // namespace cs
