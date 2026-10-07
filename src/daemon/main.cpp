// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"
#include "engine.h"
#include "kdenlivedbusclient.h"
#include "learn.h"
#include "paddevice.h"
#include "uinputsink.h"
#include "windowtracker.h"

#include <QCommandLineParser>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QSocketNotifier>
#include <QTextStream>
#include <QTimer>

#include <algorithm>
#include <cmath>
#include <csignal>
#include <memory>
#include <cstdio>
#include <sys/socket.h>
#include <unistd.h>

using namespace cs;

namespace {

int g_sigFd[2] = {-1, -1};

void onSignal(int)
{
    const char c = 1;
    [[maybe_unused]] auto r = ::write(g_sigFd[1], &c, 1);
}

void installSignalHandlers(QCoreApplication &app)
{
    if (::socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, g_sigFd) != 0) {
        return;
    }
    auto *sn = new QSocketNotifier(g_sigFd[0], QSocketNotifier::Read, &app);
    QObject::connect(sn, &QSocketNotifier::activated, &app, [&app] {
        char c;
        [[maybe_unused]] auto r = ::read(g_sigFd[0], &c, 1);
        app.quit();  // destructors release the grab and the uinput device
    });
    struct sigaction sa {};
    sa.sa_handler = onSignal;
    sigemptyset(&sa.sa_mask);
    ::sigaction(SIGINT, &sa, nullptr);
    ::sigaction(SIGTERM, &sa, nullptr);
    ::sigaction(SIGHUP, &sa, nullptr);
}

void say(const QString &s)
{
    std::printf("%s\n", qPrintable(s));
    std::fflush(stdout);
}

std::optional<Config> obtainConfig(const QString &path, bool explicitPath)
{
    QString err;
    if (QFile::exists(path)) {
        auto c = loadConfig(path, &err);
        if (!c) {
            std::fprintf(stderr, "config %s: %s\n", qPrintable(path), qPrintable(err));
        }
        return c;
    }
    if (explicitPath) {
        std::fprintf(stderr, "config %s not found\n", qPrintable(path));
        return std::nullopt;
    }
    QFile f(QStringLiteral(":/control-surface/config.example.jsonc"));
    if (!f.open(QIODevice::ReadOnly)) {
        std::fprintf(stderr, "built-in config missing\n");
        return std::nullopt;
    }
    auto c = parseConfig(f.readAll(), QFileInfo(path).absolutePath(), &err);
    if (!c) {
        std::fprintf(stderr, "built-in config: %s\n", qPrintable(err));
    } else {
        say(QStringLiteral("no %1, using the built-in example config").arg(path));
    }
    return c;
}

// simulate: "window CLASS [TITLE]" | "pid N" | "context {json}" | "kdenlive on|off"
//           "key3" | "knob1 +3" | "knob1 -1" | "knob2 press" | "wait MS" | "# comment"
int simulate(Engine &engine, FakeKdenliveClient &kd, StaticWindowTracker &tracker, QIODevice &in)
{
    QTextStream ts(&in);
    int line = 0;
    while (!ts.atEnd()) {
        const QString raw = ts.readLine();
        ++line;
        const QString l = raw.trimmed();
        if (l.isEmpty() || l.startsWith(QLatin1Char('#'))) {
            continue;
        }
        say(QStringLiteral("> ") + l);
        const QString cmd = l.section(QLatin1Char(' '), 0, 0);
        const QString rest = l.section(QLatin1Char(' '), 1);
        if (cmd == QLatin1String("window")) {
            WindowInfo w = tracker.current();
            w.cls = rest.section(QLatin1Char(' '), 0, 0);
            w.title = rest.section(QLatin1Char(' '), 1);
            w.pid = w.pid ? w.pid : 4242;
            tracker.set(w);
        } else if (cmd == QLatin1String("pid")) {
            WindowInfo w = tracker.current();
            w.pid = rest.toLongLong();
            tracker.set(w);
        } else if (cmd == QLatin1String("context")) {
            kd.setContext(QJsonDocument::fromJson(rest.toUtf8()).object().toVariantMap());
        } else if (cmd == QLatin1String("kdenlive")) {
            kd.setAvailable(rest == QLatin1String("on"));
        } else if (cmd == QLatin1String("wait")) {
            QEventLoop loop;
            QTimer::singleShot(rest.toInt(), &loop, &QEventLoop::quit);
            loop.exec();
        } else if (cmd.startsWith(QLatin1String("key"))) {
            engine.handle(PadEvent{cmd, PadEvent::KeyDown, 0, 0});
        } else if (cmd.startsWith(QLatin1String("knob"))) {
            if (rest == QLatin1String("press")) {
                engine.handle(PadEvent{cmd, PadEvent::PressDown, 0, 0});
            } else {
                const int n = rest.toInt();
                for (int i = 0; i < std::abs(n); ++i) {
                    engine.handle(PadEvent{cmd, PadEvent::Turn, n > 0 ? 1 : -1, 0});
                }
            }
        } else {
            std::fprintf(stderr, "line %d: unknown command '%s'\n", line, qPrintable(cmd));
            return 2;
        }
        // let coalescers, acks and tap pacing run
        QEventLoop loop;
        QTimer::singleShot(qMax(30, engine.config().settings.coalesceMs * 3), &loop, &QEventLoop::quit);
        loop.exec();
        while (engine.pendingTaps() > 0) {
            QTimer::singleShot(10, &loop, &QEventLoop::quit);
            loop.exec();
        }
    }
    return 0;
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    QCoreApplication::setApplicationName(QStringLiteral("control-surfaced"));
    QCoreApplication::setApplicationVersion(QStringLiteral("0.1.0"));

    QCommandLineParser p;
    p.setApplicationDescription(QStringLiteral(
        "Per-application control surface for the CH552 macro pad (1189:8890).\n"
        "Commands: run (default) | simulate [FILE|-] | verify | list-devices | check-config | example-config | bench-dbus [N]"));
    p.addHelpOption();
    p.addVersionOption();
    p.addPositionalArgument(QStringLiteral("command"), QStringLiteral("see above"));
    QCommandLineOption configOpt({QStringLiteral("c"), QStringLiteral("config")}, QStringLiteral("config file"), QStringLiteral("path"), defaultConfigPath());
    QCommandLineOption dryOpt(QStringLiteral("dry-run"), QStringLiteral("grab the pad but only print what would be sent"));
    QCommandLineOption noGrabOpt(QStringLiteral("no-grab"), QStringLiteral("read the pad without exclusive grab (debug only)"));
    QCommandLineOption backendOpt(QStringLiteral("window-backend"), QStringLiteral("auto, hyprland or none"), QStringLiteral("name"), QStringLiteral("auto"));
    QCommandLineOption serviceOpt(QStringLiteral("kdenlive-service"), QStringLiteral("always talk to this D-Bus service (e.g. the mock)"), QStringLiteral("name"));
    QCommandLineOption writeOpt(QStringLiteral("write"), QStringLiteral("verify: hardware map output"), QStringLiteral("path"), defaultHardwareMapPath());
    QCommandLineOption noWriteOpt(QStringLiteral("no-write"), QStringLiteral("verify: only report"));
    QCommandLineOption forceWindowOpt(QStringLiteral("force-window"), QStringLiteral("pretend this window class is focused (testing)"), QStringLiteral("class"));
    QCommandLineOption quietOpt({QStringLiteral("q"), QStringLiteral("quiet")}, QStringLiteral("log only problems"));
    p.addOptions({configOpt, dryOpt, noGrabOpt, backendOpt, serviceOpt, writeOpt, noWriteOpt, forceWindowOpt, quietOpt});
    p.process(app);
    const QString cmd = p.positionalArguments().value(0, QStringLiteral("run"));
    const bool explicitConfig = p.isSet(configOpt);
    installSignalHandlers(app);

    if (cmd == QLatin1String("example-config")) {
        QFile f(QStringLiteral(":/control-surface/config.example.jsonc"));
        if (!f.open(QIODevice::ReadOnly)) {
            return 1;
        }
        std::fwrite(f.readAll().constData(), 1, size_t(f.size()), stdout);
        return 0;
    }
    auto cfg = obtainConfig(p.value(configOpt), explicitConfig);
    if (!cfg) {
        return 2;
    }
    if (cmd == QLatin1String("check-config")) {
        say(QStringLiteral("ok: %1 profiles, hardware map %2 (%3 chords)").arg(cfg->profiles.size()).arg(cfg->hardwareSource).arg(cfg->hardware.size()));
        for (const auto &pr : cfg->profiles) {
            say(QStringLiteral("  profile %1: %2 layers, %3 base bindings%4").arg(pr.name).arg(pr.layers.size()).arg(pr.bindings.size()).arg(pr.kdenlive ? QStringLiteral(", kdenlive") : QString()));
        }
        return 0;
    }
    if (cmd == QLatin1String("list-devices")) {
        const auto nodes = findPadInputNodes(cfg->device);
        for (const auto &n : nodes) {
            say(QStringLiteral("%1 interface %2 serial %3 (%4)").arg(n.devnode).arg(n.interfaceNumber).arg(n.serial, n.usbPath));
        }
        return nodes.isEmpty() ? 1 : 0;
    }
    if (cmd == QLatin1String("verify")) {
        PadDevice dev(cfg->device);
        dev.setGrab(true);
        QObject::connect(&dev, &PadDevice::message, [](const QString &m) { std::fprintf(stderr, "(%s)\n", qPrintable(m)); });
        PadVerifier v(&dev, p.value(writeOpt), !p.isSet(noWriteOpt));
        QObject::connect(&v, &PadVerifier::finished, &app, [&app](int code) { app.exit(code); });
        v.start();
        return app.exec();
    }
    if (cmd == QLatin1String("simulate")) {
        RecordingKeySink keys(true);
        FakeKdenliveClient kd(true);
        StaticWindowTracker tracker;
        Engine engine(&keys, &kd);
        engine.setConfig(*cfg);
        QObject::connect(&tracker, &WindowTracker::activeWindowChanged, &engine, &Engine::setActiveWindow);
        QObject::connect(&engine, &Engine::message, [](const QString &m) { say(QStringLiteral("  (%1)").arg(m)); });
        QObject::connect(&engine, &Engine::runCommand, [](const QStringList &a) { say(QStringLiteral("  -> command %1").arg(a.join(QLatin1Char(' ')))); });
        const QString file = p.positionalArguments().value(1, QStringLiteral("-"));
        QFile in;
        if (file == QLatin1String("-")) {
            if (!in.open(stdin, QIODevice::ReadOnly)) {
                return 2;
            }
        } else {
            in.setFileName(file);
            if (!in.open(QIODevice::ReadOnly)) {
                std::fprintf(stderr, "cannot read %s\n", qPrintable(file));
                return 2;
            }
        }
        return simulate(engine, kd, tracker, in);
    }
    if (cmd == QLatin1String("bench-dbus")) {
        // Round trip Control -> ControlAck against a contract implementation.
        const QString service = p.value(serviceOpt);
        if (service.isEmpty()) {
            std::fprintf(stderr, "bench-dbus needs --kdenlive-service NAME\n");
            return 2;
        }
        KdenliveDBusClient client(QDBusConnection::sessionBus());
        client.setServiceOverride(service);
        client.attachToPid(1);
        QEventLoop loop;
        QTimer::singleShot(2000, &loop, &QEventLoop::quit);
        QObject::connect(&client, &KdenliveClient::availabilityChanged, &loop, &QEventLoop::quit);
        loop.exec();
        if (!client.isAvailable()) {
            std::fprintf(stderr, "%s does not implement %s\n", qPrintable(service), "org.kde.kdenlive.ControlSurface1");
            return 3;
        }
        const int n = p.positionalArguments().value(1, QStringLiteral("500")).toInt();
        QList<double> us;
        QElapsedTimer t;
        QTimer deadline;
        deadline.setSingleShot(true);
        QObject::connect(&deadline, &QTimer::timeout, &loop, &QEventLoop::quit);
        bool acked = false;
        QObject::connect(&client, &KdenliveClient::controlAcked, &loop, [&] {
            acked = true;
            loop.quit();
        });
        for (int i = 0; i < n; ++i) {
            acked = false;
            t.start();
            client.control(QStringLiteral("playhead.jog"), (i % 2) ? -1 : 1, {});
            deadline.start(500);
            loop.exec();
            deadline.stop();
            if (acked) {
                us << t.nsecsElapsed() / 1000.0;
            }
        }
        std::sort(us.begin(), us.end());
        if (us.isEmpty()) {
            std::fprintf(stderr, "no acknowledgements received\n");
            return 4;
        }
        say(QStringLiteral("%1/%2 acked; round trip us: p50 %3  p90 %4  p99 %5  max %6")
                .arg(us.size())
                .arg(n)
                .arg(us.at(us.size() / 2), 0, 'f', 0)
                .arg(us.at(us.size() * 9 / 10), 0, 'f', 0)
                .arg(us.at(qMin(us.size() - 1, us.size() * 99 / 100)), 0, 'f', 0)
                .arg(us.last(), 0, 'f', 0));
        client.attachToPid(0);
        QTimer::singleShot(100, &loop, &QEventLoop::quit);
        loop.exec();
        return 0;
    }
    if (cmd != QLatin1String("run")) {
        p.showHelp(2);
    }

    const bool dry = p.isSet(dryOpt);
    const bool quiet = p.isSet(quietOpt);
    auto log = [quiet](const QString &m) {
        if (!quiet) {
            say(m);
        }
    };

    std::unique_ptr<KeySink> keys;
    if (dry) {
        keys = std::make_unique<RecordingKeySink>(true);
    } else {
        auto u = std::make_unique<UinputKeySink>();
        QString err;
        if (!u->open(&err)) {
            std::fprintf(stderr, "%s\n", qPrintable(err));
            return 3;
        }
        keys = std::move(u);
    }
    std::unique_ptr<KdenliveClient> kd;
    if (dry) {
        kd = std::make_unique<FakeKdenliveClient>(true);
    } else {
        auto c = std::make_unique<KdenliveDBusClient>(QDBusConnection::sessionBus());
        if (p.isSet(serviceOpt)) {
            c->setServiceOverride(p.value(serviceOpt));
        }
        QObject::connect(c.get(), &KdenliveClient::message, log);
        kd = std::move(c);
    }
    Engine engine(keys.get(), kd.get());
    engine.setConfig(*cfg);
    QObject::connect(&engine, &Engine::message, log);
    QObject::connect(&engine, &Engine::runCommand, [dry](const QStringList &a) {
        if (dry) {
            say(QStringLiteral("  -> command %1").arg(a.join(QLatin1Char(' '))));
        } else if (!a.isEmpty()) {
            QProcess::startDetached(a.first(), a.mid(1));
        }
    });

    std::unique_ptr<WindowTracker> tracker;
    if (p.isSet(forceWindowOpt)) {
        auto st = std::make_unique<StaticWindowTracker>();
        st->set(WindowInfo{p.value(forceWindowOpt), QString(), 1, QString()});
        tracker = std::move(st);
        engine.setActiveWindow(tracker->current());
    } else {
        tracker = createWindowTracker(p.value(backendOpt));
    }
    QObject::connect(tracker.get(), &WindowTracker::message, log);
    QObject::connect(tracker.get(), &WindowTracker::activeWindowChanged, &engine, [&engine, log](const WindowInfo &w) {
        engine.setActiveWindow(w);
        log(QStringLiteral("focus: %1 \"%2\" pid %3").arg(w.cls, w.title.left(60)).arg(w.pid));
    });
    log(QStringLiteral("window backend: %1, hardware map: %2").arg(tracker->backendName(), cfg->hardwareSource));
    tracker->start();

    PadDevice dev(cfg->device);
    dev.setHardwareMap(cfg->hardware);
    dev.setGrab(!p.isSet(noGrabOpt));
    QObject::connect(&dev, &PadDevice::message, log);
    QObject::connect(&dev, &PadDevice::connected, [log](const QStringList &n) { log(QStringLiteral("pad connected: %1").arg(n.join(QStringLiteral(", ")))); });
    QObject::connect(&dev, &PadDevice::disconnected, [log] { log(QStringLiteral("pad disconnected, waiting")); });
    QObject::connect(&dev, &PadDevice::unmappedChord, [log](const KeyChord &c) { log(QStringLiteral("unmapped chord %1 (run 'control-surfaced verify')").arg(chordName(c))); });
    QObject::connect(&dev, &PadDevice::padEvent, &engine, [&engine, dry](const PadEvent &e) {
        if (dry) {
            const auto slot = e.type == PadEvent::Turn ? e.control + QStringLiteral(".turn") : e.type == PadEvent::PressDown ? e.control + QStringLiteral(".press") : e.control;
            if (e.type != PadEvent::KeyUp && e.type != PadEvent::PressUp) {
                const auto r = engine.resolve(slot);
                say(QStringLiteral("%1 -> %2%3").arg(e.describe(), r ? r->binding.describe() : QStringLiteral("(unbound)"),
                                                      r && !r->layer.isEmpty() ? QStringLiteral(" [layer %1]").arg(r->layer) : QString()));
            }
        }
        engine.handle(e);
    });
    dev.start();
    const int rc = app.exec();
    dev.stop();
    return rc;
}
