// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "boardprofile.h"
#include "configstore.h"
#include "decoder.h"
#include "flashjob.h"
#include "usbinfo.h"

#include <QDBusConnection>
#include <QDBusContext>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <functional>
#include <optional>

class QDBusServiceWatcher;
class QTimer;

namespace cs {

// What the daemon knows about the pad right now.
struct DeviceState {
    bool present = false;
    UsbDeviceInfo usb;
    FirmwareInfo firmware;
    QStringList devnodes;
    QString inputMode;  // evdev-chords | raw | "" (absent)
    QJsonObject toJson() const;
};

// One entry of ListPlugins (docs/GENERALIZATION-PLAN.md (c)).
struct PluginInfo {
    QString id;      // keys | command | kdenlive | ...
    QString name;
    QString tier;    // keys | command | api
    QString status;  // ready | available | absent | pending | disabled | dry-run
    QString detail;
    QStringList apps;  // window classes the plugin serves (empty: all)
    QJsonObject extra;
    QJsonObject toJson() const;
};

// org.smplos.ControlSurface1 on the session bus: the API the smplOS Settings
// "Keypad" app and the bar icon use. Reference: docs/dbus-settings-api.md.
// The daemon (and the mock) push state in; the service answers queries,
// publishes live input, runs identify mode, guards config writes and owns the
// one flash job.
class SettingsService : public QObject, protected QDBusContext
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.smplos.ControlSurface1")
    Q_PROPERTY(uint ApiVersion READ apiVersion)
    Q_PROPERTY(QString DaemonVersion READ daemonVersion)
    Q_PROPERTY(QString Mode READ mode)
    Q_PROPERTY(bool DevicePresent READ devicePresent)
    Q_PROPERTY(QString FirmwareType READ firmwareType)
    Q_PROPERTY(QString ActiveProfile READ activeProfile)
    Q_PROPERTY(QString ActiveLayer READ activeLayer)
    Q_PROPERTY(QString ConfigHash READ configHash)
    Q_PROPERTY(bool Identifying READ identifyActive)

public:
    static constexpr const char *kService = "org.smplos.ControlSurface";
    static constexpr const char *kPath = "/org/smplos/ControlSurface";
    static constexpr const char *kInterface = "org.smplos.ControlSurface1";
    static constexpr uint kApiVersion = 1;
    static constexpr int kIdentifySafetyMs = 10 * 60 * 1000;  // SetIdentify(true) again renews it

    explicit SettingsService(const QString &configPath, QObject *parent = nullptr);

    // Exports the object; with claimName also requests org.smplos.ControlSurface.
    bool registerOn(const QDBusConnection &bus, bool claimName, QString *error);

    void setMode(const QString &mode);
    void setDaemonVersion(const QString &version) { m_version = version; }
    void setDevice(const DeviceState &d);
    void setActiveProfile(const QString &name);
    void setActiveLayer(const QString &name);
    void setActiveWindow(const QString &cls, const QString &title);
    void setPlugins(const QList<PluginInfo> &plugins);
    // After the daemon (re)loaded the config by itself (start-up, file watcher).
    void setConfigState(const QString &hash, const QString &error, const QStringList &warnings);
    void setFallbackLayout(const BoardProfile &p) { m_fallbackLayout = p; }
    using ConfigApplier = std::function<QString(const Config &)>;  // error, or empty when applied
    void setConfigApplier(ConfigApplier a) { m_apply = std::move(a); }
    void setFlashSettings(const FlashSettings &s) { m_flash = s; }
    using JobSetup = std::function<void(FlashJob *)>;  // install probe and runner (mock, tests)
    void setJobSetup(JobSetup f) { m_jobSetup = std::move(f); }

    // Pad input from the daemon: always published as InputEvent. Returns true
    // while identify mode suppresses actions; the caller must then not dispatch it.
    bool filterPadEvent(const PadEvent &e);
    // InputEvent arguments for a pad event: slot key7/knob2, event press|release|ccw|cw, delta.
    static void inputEventFor(const PadEvent &e, QString *slot, QString *event, int *delta);

    uint apiVersion() const { return kApiVersion; }
    QString daemonVersion() const { return m_version; }
    QString mode() const { return m_mode; }
    bool devicePresent() const { return m_device.present; }
    QString firmwareType() const { return m_device.present ? m_device.firmware.type : QString(); }
    QString activeProfile() const { return m_profile; }
    QString activeLayer() const { return m_layer; }
    QString configHash() const { return m_configHash; }
    bool identifyActive() const;
    const DeviceState &device() const { return m_device; }
    FlashJob *currentJob() const { return m_job; }

public Q_SLOTS:
    Q_SCRIPTABLE QString GetStatus();
    Q_SCRIPTABLE QString GetDevice();
    Q_SCRIPTABLE QString GetLayout();
    Q_SCRIPTABLE QString ListBoardProfiles();
    Q_SCRIPTABLE void SetIdentify(bool on);
    Q_SCRIPTABLE QString GetConfig(QString &path, QString &hash);
    Q_SCRIPTABLE QString ValidateConfig(const QString &text);
    Q_SCRIPTABLE QString SetConfig(const QString &text, const QString &expectedHash);
    Q_SCRIPTABLE QString ReloadConfig();
    Q_SCRIPTABLE QString ListPlugins();
    Q_SCRIPTABLE QString GetFirmwareStatus();
    Q_SCRIPTABLE QString StartFlash(const QString &image, const QString &sha256, bool dryRun);
    Q_SCRIPTABLE bool CancelFlash(const QString &jobId);

Q_SIGNALS:
    Q_SCRIPTABLE void InputEvent(const QString &slot, const QString &event, int delta);
    Q_SCRIPTABLE void IdentifyChanged(bool on);
    Q_SCRIPTABLE void DeviceChanged(bool present, const QString &firmwareType);
    Q_SCRIPTABLE void ConfigChanged(const QString &hash);
    Q_SCRIPTABLE void ConfigRejected(const QString &error);
    Q_SCRIPTABLE void PluginsChanged();
    Q_SCRIPTABLE void FlashProgress(const QString &jobId, const QString &phase, const QString &message);
    // In-process only: the flash job wants the pad released / grabbed again.
    void releaseDeviceRequested();
    void reacquireDeviceRequested();

private:
    void propertiesChanged(const QVariantMap &changed);
    void endIdentify();
    QString applyText(const QByteArray &text, const QString &hash, QJsonObject *out);
    QJsonObject layoutJson() const;
    static QString json(const QJsonObject &o);

    ConfigStore m_store;
    QDBusConnection m_bus;
    bool m_registered = false;
    QString m_mode = QStringLiteral("run");
    QString m_version;
    DeviceState m_device;
    QString m_profile, m_layer, m_windowClass, m_windowTitle;
    QList<PluginInfo> m_plugins;
    QString m_configHash, m_configError;
    QStringList m_configWarnings;
    std::optional<BoardProfile> m_fallbackLayout;
    ConfigApplier m_apply;
    FlashSettings m_flash;
    JobSetup m_jobSetup;
    QPointer<FlashJob> m_job;
    int m_jobCounter = 0;
    QTimer *m_identifyTimer;
    QString m_identifyOwner;
    QDBusServiceWatcher *m_ownerWatch;
};

} // namespace cs
