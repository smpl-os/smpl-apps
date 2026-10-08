// SPDX-License-Identifier: GPL-2.0-or-later
#include "mocksurface.h"

#include <cstdio>

#include <QCoreApplication>
#include <QDBusError>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTimer>

namespace cs {

namespace {
Config loadOrEmpty(const QString &path, QStringList *warnings)
{
    QString err;
    if (auto c = loadConfig(path, &err)) {
        return *c;
    }
    if (warnings) {
        *warnings << QStringLiteral("mock config %1: %2").arg(path, err);
    }
    return Config{};
}

} // namespace

MockSurface::MockSurface(const Options &o, QObject *parent)
    : QObject(parent)
    , m_o(o)
    , m_keys(std::make_unique<RecordingKeySink>(false))
    , m_kd(std::make_unique<FakeKdenliveClient>(false))
    , m_firmware(o.firmware)
    , m_board(o.board)
{
    QStringList problems;
    const Config cfg = loadOrEmpty(o.configPath, &problems);
    m_engine = std::make_unique<Engine>(m_keys.get(), m_kd.get());
    m_engine->setConfig(cfg);
    m_engine->setOneAtATime(effectiveLayout(cfg).oneAtATime);
    m_settings = std::make_unique<SettingsService>(o.configPath);
    m_settings->setMode(QStringLiteral("mock"));
    m_settings->setDaemonVersion(QStringLiteral("mock-") + QCoreApplication::applicationVersion());
    m_settings->setFallbackLayout(effectiveLayout(cfg));
    m_settings->setConfigState(ConfigStore::hashOf(ConfigStore(o.configPath).read().text), problems.join(QStringLiteral("; ")), cfg.warnings);
    m_settings->setBoardWarnings([this](const BoardProfile &l) { return boardWarnings(m_engine->config(), l); });
    m_settings->setConfigApplier([this](const Config &c) {
        m_engine->setConfig(c);
        m_engine->setOneAtATime(effectiveLayout(c).oneAtATime);
        m_settings->setFallbackLayout(effectiveLayout(c));
        if (m_eww) {
            m_eww->setOptions(c.cheatsheet.eww.over(m_o.eww));
        }
        return QString();
    });

    FlashSettings fs;
    fs.allowed = true;  // simulated: the runner below never starts a process
    fs.tool = QCoreApplication::applicationFilePath();
    fs.imageDirs = {o.firmwareDir};
    fs.bootloaderTimeoutMs = 120000;
    fs.deviceTimeoutMs = 10000;
    fs.pollMs = 50;
    m_settings->setFlashSettings(fs);
    m_settings->setJobSetup([this](FlashJob *job) {
        job->setProbe([this] { return usbDevices(); });
        job->setRunner([this](const QString &image, FlashJob::Done done) {
            QTimer::singleShot(m_o.flashStepMs, this, [this, image, done] {
                if (m_flashOutcome == QLatin1String("tool-fails")) {
                    done(1, QStringLiteral("mock: simulated tool failure"));
                    return;
                }
                m_bootloader = false;
                m_firmware = QStringLiteral("control-surface");
                m_plugged = m_flashOutcome != QLatin1String("no-return");
                publishDevice();
                done(0, QStringLiteral("mock: erased, wrote and verified %1").arg(image));
            });
        });
    });

    m_cheatsheet = std::make_unique<Cheatsheet>(m_engine.get(), m_kd.get());
    m_settings->setCheatsheet(m_cheatsheet.get());
    m_eww = std::make_unique<EwwSink>(m_cheatsheet.get());
    connect(m_eww.get(), &EwwSink::message, this, [](const QString &m) { std::fprintf(stderr, "%s\n", qPrintable(m)); });
    m_eww->setOptions(cfg.cheatsheet.eww.over(o.eww));
    m_settings->setCheatsheetStatus([this] { return QJsonObject{{QStringLiteral("eww"), m_eww->status()}}; });
    connect(m_engine.get(), &Engine::dispatched, this, [this](const QString &slot, const QString &binding, const QString &layer) {
        m_settings->setActiveLayer(layer);
        Q_EMIT Dispatched(slot, binding, layer);
    });
    connect(m_kd.get(), &KdenliveClient::stateChanged, this, &MockSurface::publishPlugins);

    m_plugged = o.plugged;
    publishDevice();
    publishPlugins();
    m_settings->setActiveProfile(m_engine->activeProfile() ? m_engine->activeProfile()->name : QString());
}

MockSurface::~MockSurface()
{
    m_cheatsheet->hide();
    m_eww->finish();  // eww shows it hidden before we go
    m_eww.reset();
    m_cheatsheet.reset();  // before the engine and the service it talks to
}

bool MockSurface::registerOn(const QDBusConnection &bus, bool claimName, QString *error)
{
    if (!m_settings->registerOn(bus, claimName, error)) {
        return false;
    }
    QDBusConnection b(bus);
    if (!b.registerObject(QLatin1String(kMockPath), this, QDBusConnection::ExportScriptableSlots | QDBusConnection::ExportScriptableSignals)) {
        if (error) {
            *error = QStringLiteral("cannot export %1").arg(QLatin1String(kMockPath));
        }
        return false;
    }
    return true;
}

UsbDeviceInfo MockSurface::padUsb() const
{
    UsbDeviceInfo d;
    d.sysPath = QStringLiteral("/mock/usb/1-4");
    d.vendor = QStringLiteral("1189");
    d.product = QStringLiteral("8890");
    d.serial = QStringLiteral("key153");
    d.busnum = 1;
    d.devnum = 30;
    if (m_firmware == QLatin1String("control-surface")) {
        d.manufacturer = QStringLiteral("OpenMacroPad");
        d.productName = QStringLiteral("Control Surface 15+3");
        d.bcdDevice = QStringLiteral("0200");
        d.interfaces = 1;
    } else if (m_firmware == QLatin1String("openmacropad")) {
        d.manufacturer = QStringLiteral("SY181");
        d.productName = QStringLiteral("Macropad 12+3");
        d.bcdDevice = QStringLiteral("0100");
        d.interfaces = 1;
    } else {
        d.manufacturer = QStringLiteral("wch.cn");
        d.productName = QStringLiteral("CH552");
        d.bcdDevice = QStringLiteral("0100");
        d.interfaces = 4;
    }
    return d;
}

QList<UsbDeviceInfo> MockSurface::usbDevices() const
{
    QList<UsbDeviceInfo> out;
    if (m_plugged) {
        out << padUsb();
    }
    if (m_bootloader) {
        UsbDeviceInfo b;
        b.sysPath = QStringLiteral("/mock/usb/1-4");
        b.vendor = QStringLiteral("4348");
        b.product = QStringLiteral("55e0");
        b.busnum = 1;
        b.devnum = m_bootloaderDevnum;
        out << b;
    }
    return out;
}

void MockSurface::publishDevice()
{
    DeviceState d;
    if (m_plugged) {
        d.present = true;
        d.usb = padUsb();
        d.firmware = classifyFirmware(d.usb);
        d.firmware.board = m_board;  // what CMD_IDENTIFY will report on real pads
        if (d.firmware.type == QLatin1String("control-surface")) {
            // As the daemon reports it after GET_INFO (bcdDevice has only 2.0).
            d.firmware.version = QStringLiteral("2.0.1");
            d.firmware.versionSource = QStringLiteral("GET_INFO");
            const auto board = builtinBoardProfile(m_board);
            d.firmware.slotCount = board ? board->slotCount() : 24;
        }
        d.devnodes = {QStringLiteral("/dev/input/mock-event-kbd")};
        d.inputMode = QStringLiteral("evdev-chords");
    }
    m_settings->setDevice(d);
}

void MockSurface::publishPlugins()
{
    static const char *states[] = {"detached", "pending", "absent", "available"};
    auto status = [this](const QString &id, const QString &fallback) { return m_pluginStatus.value(id, fallback); };
    QStringList kdApps;
    for (const Profile &pr : m_engine->config().profiles) {
        if (pr.kdenlive && pr.hasMatch) {
            kdApps << pr.matchClass.pattern();
        }
    }
    m_settings->setPlugins({
        PluginInfo{QStringLiteral("keys"), QStringLiteral("Keys and shortcuts"), QStringLiteral("keys"), status(QStringLiteral("keys"), QStringLiteral("ready")),
                   QStringLiteral("mock: keys are recorded, never sent"), {}, {}},
        PluginInfo{QStringLiteral("command"), QStringLiteral("Run a program"), QStringLiteral("command"), status(QStringLiteral("command"), QStringLiteral("ready")),
                   QStringLiteral("mock: commands are not run"), {}, {}},
        PluginInfo{QStringLiteral("kdenlive"), QStringLiteral("Kdenlive"), QStringLiteral("api"),
                   status(QStringLiteral("kdenlive"), QLatin1String(states[int(m_kd->state())])), QStringLiteral("mock"), kdApps,
                   QJsonObject{{QStringLiteral("contract"), QStringLiteral("org.kde.kdenlive.ControlSurface1")}}},
    });
}

void MockSurface::event(const QString &control, PadEvent::Type type, int delta)
{
    static const QRegularExpression valid(QStringLiteral("^(key([1-9]|1[0-6])|knob[1-3])$"));
    if (!valid.match(control).hasMatch()) {
        if (calledFromDBus()) {
            sendErrorReply(QDBusError::InvalidArgs, QStringLiteral("unknown control %1 (key1..key16, knob1..knob3)").arg(control));
        }
        return;
    }
    if (!m_plugged) {
        return;  // nothing is connected
    }
    const PadEvent e{control, type, delta, 0};
    if (!m_settings->filterPadEvent(e)) {
        m_engine->handle(e);
    } else if (type == PadEvent::KeyUp || type == PadEvent::PressUp) {
        m_engine->handle(PadEvent{control, type, delta, 0, true});  // as the daemon: no stuck hold
    }
}

void MockSurface::Plug(const QString &firmwareType, const QString &board)
{
    m_firmware = firmwareType.isEmpty() ? QStringLiteral("control-surface") : firmwareType;
    if (!board.isEmpty()) {
        m_board = board;
    }
    m_plugged = true;
    m_bootloader = false;
    publishDevice();
}

void MockSurface::Unplug()
{
    m_plugged = false;
    m_bootloader = false;
    m_engine->releaseAll();  // as the daemon: nothing stays held (held layers)
    publishDevice();
}

void MockSurface::Press(const QString &control)
{
    const bool knob = control.startsWith(QLatin1String("knob"));
    event(control, knob ? PadEvent::PressDown : PadEvent::KeyDown, 0);
    event(control, knob ? PadEvent::PressUp : PadEvent::KeyUp, 0);
}

void MockSurface::Hold(const QString &control)
{
    event(control, control.startsWith(QLatin1String("knob")) ? PadEvent::PressDown : PadEvent::KeyDown, 0);
}

void MockSurface::Release(const QString &control)
{
    event(control, control.startsWith(QLatin1String("knob")) ? PadEvent::PressUp : PadEvent::KeyUp, 0);
}

void MockSurface::Turn(const QString &knob, int detents)
{
    if (!knob.startsWith(QLatin1String("knob")) || detents == 0 || detents < -100 || detents > 100) {
        if (calledFromDBus()) {
            sendErrorReply(QDBusError::InvalidArgs, QStringLiteral("Turn needs knob1..knob3 and 1..100 detents either way"));
        }
        return;
    }
    for (int i = 0; i < qAbs(detents); ++i) {
        event(knob, PadEvent::Turn, detents > 0 ? 1 : -1);
    }
}

void MockSurface::Focus(const QString &windowClass, const QString &title)
{
    m_engine->setActiveWindow(WindowInfo{windowClass, title, 4242, QString()});
    m_settings->setActiveWindow(windowClass, title);
    m_settings->setActiveProfile(m_engine->activeProfile() ? m_engine->activeProfile()->name : QString());
}

void MockSurface::SetPluginStatus(const QString &id, const QString &status)
{
    if (status.isEmpty()) {
        m_pluginStatus.remove(id);
    } else {
        m_pluginStatus.insert(id, status);
    }
    publishPlugins();
}

void MockSurface::EnterBootloader()
{
    m_plugged = false;
    m_bootloader = true;
    ++m_bootloaderDevnum;  // a fresh session, as after a real replug
    publishDevice();
}

void MockSurface::SetFlashOutcome(const QString &outcome)
{
    static const QStringList known{QStringLiteral("ok"), QStringLiteral("tool-fails"), QStringLiteral("no-return")};
    if (!known.contains(outcome)) {
        if (calledFromDBus()) {
            sendErrorReply(QDBusError::InvalidArgs, QStringLiteral("outcome must be one of %1").arg(known.join(QStringLiteral(", "))));
        }
        return;
    }
    m_flashOutcome = outcome;
}

QStringList MockSurface::TakeKeys()
{
    const QStringList t = m_keys->taps;
    m_keys->taps.clear();
    return t;
}

void MockSurface::SetKdenliveState(const QString &state)
{
    static const QHash<QString, KdenliveClient::State> states{{QStringLiteral("available"), KdenliveClient::State::Available},
                                                               {QStringLiteral("absent"), KdenliveClient::State::Absent},
                                                               {QStringLiteral("pending"), KdenliveClient::State::Pending},
                                                               {QStringLiteral("detached"), KdenliveClient::State::Detached}};
    if (!states.contains(state)) {
        if (calledFromDBus()) {
            sendErrorReply(QDBusError::InvalidArgs, QStringLiteral("state must be available, absent, pending or detached"));
        }
        return;
    }
    m_kd->setState(states.value(state));
}

void MockSurface::SetKdenliveContext(const QString &json)
{
    const QJsonDocument d = QJsonDocument::fromJson(json.toUtf8());
    if (!d.isObject()) {
        if (calledFromDBus()) {
            sendErrorReply(QDBusError::InvalidArgs, QStringLiteral("the context must be a JSON object"));
        }
        return;
    }
    m_kd->setContext(d.object().toVariantMap());
}

} // namespace cs
