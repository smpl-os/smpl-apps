// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "engine.h"
#include "kdenliveclient.h"
#include "keysink.h"
#include "settingsservice.h"

#include <QDBusConnection>
#include <QObject>
#include <memory>

namespace cs {

// A stand-in for control-surfaced for UI development: serves the real
// org.smplos.ControlSurface1 (SettingsService) with a simulated pad, plus
// org.smplos.ControlSurface1.Mock at /org/smplos/ControlSurface/Mock to plug,
// press, turn and walk the flash wizard. Nothing touches hardware: input is
// dispatched by a real Engine into a recording key sink and a fake Kdenlive.
class MockSurface : public QObject, protected QDBusContext
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.smplos.ControlSurface1.Mock")
public:
    static constexpr const char *kMockPath = "/org/smplos/ControlSurface/Mock";

    struct Options {
        QString configPath;   // the mock's own config file (never the user's by default)
        QString firmwareDir;  // simulated images live here
        QString board = QStringLiteral("sy181-15k3e");
        QString firmware = QStringLiteral("control-surface");
        bool plugged = true;
        int flashStepMs = 400;  // pacing of the simulated flash
    };

    explicit MockSurface(const Options &o, QObject *parent = nullptr);
    ~MockSurface() override;
    bool registerOn(const QDBusConnection &bus, bool claimName, QString *error);
    SettingsService &settings() { return *m_settings; }
    Engine &engine() { return *m_engine; }
    RecordingKeySink &keys() { return *m_keys; }

    // Simulated USB devices, as the flash job's probe sees them.
    QList<UsbDeviceInfo> usbDevices() const;

public Q_SLOTS:
    Q_SCRIPTABLE void Plug(const QString &firmwareType, const QString &board);
    Q_SCRIPTABLE void Unplug();
    Q_SCRIPTABLE void Press(const QString &control);    // down then up
    Q_SCRIPTABLE void Hold(const QString &control);
    Q_SCRIPTABLE void Release(const QString &control);
    Q_SCRIPTABLE void Turn(const QString &knob, int detents);
    Q_SCRIPTABLE void Focus(const QString &windowClass, const QString &title);
    Q_SCRIPTABLE void SetPluginStatus(const QString &id, const QString &status);
    Q_SCRIPTABLE void EnterBootloader();                // what the user does with the boot key
    Q_SCRIPTABLE void SetFlashOutcome(const QString &outcome);  // ok | tool-fails | no-return
    Q_SCRIPTABLE QStringList TakeKeys();                // recorded key taps since the last call

Q_SIGNALS:
    Q_SCRIPTABLE void Dispatched(const QString &slot, const QString &binding, const QString &layer);

private:
    void event(const QString &control, PadEvent::Type type, int delta);
    void publishDevice();
    void publishPlugins();
    UsbDeviceInfo padUsb() const;

    Options m_o;
    std::unique_ptr<RecordingKeySink> m_keys;
    std::unique_ptr<FakeKdenliveClient> m_kd;
    std::unique_ptr<Engine> m_engine;
    std::unique_ptr<SettingsService> m_settings;
    bool m_plugged = false;
    bool m_bootloader = false;
    int m_bootloaderDevnum = 40;
    QString m_firmware, m_board;
    QString m_flashOutcome = QStringLiteral("ok");
    QHash<QString, QString> m_pluginStatus;
};

} // namespace cs
