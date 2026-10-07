// SPDX-License-Identifier: GPL-2.0-or-later
#include "capabilities.h"
#include "config.h"
#include "configwatcher.h"
#include "engine.h"
#include "kdenlivedbusclient.h"
#include "learn.h"
#include "paddevice.h"
#include "uinputsink.h"
#include "windowtracker.h"

#include <QCommandLineParser>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCall>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QSocketNotifier>
#include <QTextStream>
#include <QTimer>

#include <algorithm>
#include <cmath>
#include <csignal>
#include <functional>
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

// One desktop notification (org.freedesktop.Notifications); fire and forget.
void desktopNotify(const QString &body)
{
    auto msg = QDBusMessage::createMethodCall(QStringLiteral("org.freedesktop.Notifications"), QStringLiteral("/org/freedesktop/Notifications"),
                                              QStringLiteral("org.freedesktop.Notifications"), QStringLiteral("Notify"));
    msg << QStringLiteral("control-surface") << uint(0) << QStringLiteral("input-keyboard") << QStringLiteral("Control surface") << body << QStringList{}
        << QVariantMap{} << int(10000);
    QDBusConnection::sessionBus().asyncCall(msg, 2000);
}

struct SimEnv {
    Engine &engine;
    KdenliveClient &kd;
    FakeKdenliveClient *fake;  // null when driving a real Kdenlive over D-Bus
    StaticWindowTracker &tracker;
    RecordingKeySink &keys;
    QList<QPair<QString, QString>> refusals;  // (what, code) since the last check
    QStringList notices;                       // since the last "expect notice"
    int failures = 0;
};

bool waitUntil(const std::function<bool()> &done, int ms)
{
    QElapsedTimer t;
    t.start();
    while (!done()) {
        if (t.elapsed() >= ms) {
            return false;
        }
        QEventLoop loop;
        QTimer::singleShot(10, &loop, &QEventLoop::quit);
        loop.exec();
    }
    return true;
}

QVariant parseValue(const QString &text)
{
    // JSON scalars (true, 3, "x"); anything else is a string ("/regex/", "!x").
    const QJsonDocument d = QJsonDocument::fromJson(QByteArray("[") + text.toUtf8() + "]");
    return d.isArray() && d.array().size() == 1 ? d.array().at(0).toVariant() : QVariant(text);
}

QString toJson(const QVariant &v)
{
    return QString::fromUtf8(QJsonDocument(QJsonArray{QJsonValue::fromVariant(v)}).toJson(QJsonDocument::Compact)).mid(1).chopped(1);
}

void check(SimEnv &env, bool ok, const QString &what)
{
    say(QStringLiteral("  EXPECT %1: %2").arg(ok ? QStringLiteral("ok") : QStringLiteral("FAIL"), what));
    env.failures += ok ? 0 : 1;
}

// simulate: "window CLASS [TITLE]" | "pid N" | "key3" | "knob1 +3" | "knob1 -1" |
//           "knob2 press" (down + up) | "knob2 hold" | "knob2 release" |
//           "wait MS" | "# comment"
// Fake client only:   "context {json}" | "kdenlive on|off|pending" | "stage 1|2|3"
// Any client:         "await available|absent [MS]" | "await ctx PATH VALUE [MS]" |
//                     "print ctx [PATH]" | "expect refused CODE [MS]" |
//                     "expect no-refusal" | "expect no-keys" | "expect keys K1 K2 ..." |
//                     "expect notice [TEXT]" | "expect no-notice" | "refusals clear"
int simulate(SimEnv &env, QIODevice &in)
{
    QTextStream ts(&in);
    int line = 0;
    while (!ts.atEnd()) {
        const QString raw = ts.readLine();
        ++line;
        const QString l = raw.trimmed();
        if (l.isEmpty() || l.startsWith(QLatin1Char('#'))) {
            if (l.startsWith(QLatin1String("##"))) {
                say(l);  // section headings in acceptance scripts
            }
            continue;
        }
        say(QStringLiteral("> ") + l);
        const QStringList w = l.split(QLatin1Char(' '), Qt::SkipEmptyParts);
        const QString cmd = w.value(0);
        const QString rest = l.section(QLatin1Char(' '), 1);
        bool settle = true;
        if (cmd == QLatin1String("window")) {
            WindowInfo win = env.tracker.current();
            win.cls = rest.section(QLatin1Char(' '), 0, 0);
            win.title = rest.section(QLatin1Char(' '), 1);
            win.pid = win.pid ? win.pid : 4242;
            env.tracker.set(win);
        } else if (cmd == QLatin1String("pid")) {
            WindowInfo win = env.tracker.current();
            win.pid = rest.toLongLong();
            env.tracker.set(win);
        } else if (cmd == QLatin1String("context") || cmd == QLatin1String("kdenlive") || cmd == QLatin1String("stage")) {
            if (!env.fake) {
                std::fprintf(stderr, "line %d: '%s' needs the fake client (no --kdenlive-service)\n", line, qPrintable(cmd));
                return 2;
            }
            if (cmd == QLatin1String("context")) {
                env.fake->setContext(QJsonDocument::fromJson(rest.toUtf8()).object().toVariantMap());
            } else if (cmd == QLatin1String("kdenlive")) {
                env.fake->setState(rest == QLatin1String("on") ? KdenliveClient::State::Available
                                   : rest == QLatin1String("pending") ? KdenliveClient::State::Pending
                                                                      : KdenliveClient::State::Absent);
            } else {
                const int n = rest.toInt();
                QStringList controls{QStringLiteral("playhead.jog"), QStringLiteral("playhead.shuttle"), QStringLiteral("timeline.zoom")};
                QStringList commands;
                if (n >= 2) {
                    controls << QStringLiteral("param.focus") << QStringLiteral("param.nudge") << QStringLiteral("colorwheel.nudge");
                    commands << QStringLiteral("param.reset") << QStringLiteral("colorwheel.reset");
                }
                if (n >= 3) {
                    controls << QStringLiteral("timeline.track") << QStringLiteral("timeline.scroll") << QStringLiteral("audio.gain") << QStringLiteral("edit.trim");
                    commands << QStringLiteral("track.set");
                }
                env.fake->setControlCapabilities(controls, commands);
            }
        } else if (cmd == QLatin1String("await") && w.value(1) == QLatin1String("available")) {
            settle = false;
            check(env, waitUntil([&] { return env.kd.isAvailable(); }, w.value(2, QStringLiteral("5000")).toInt()), QStringLiteral("interface available"));
        } else if (cmd == QLatin1String("await") && w.value(1) == QLatin1String("absent")) {
            settle = false;
            check(env, waitUntil([&] { return env.kd.isAbsent(); }, w.value(2, QStringLiteral("5000")).toInt()), QStringLiteral("interface absent (stock Kdenlive)"));
        } else if (cmd == QLatin1String("await") && w.value(1) == QLatin1String("ctx")) {
            settle = false;
            const QString path = w.value(2);
            const QVariant want = parseValue(w.value(3));
            const bool ok = waitUntil([&] { return conditionMatches({{path, want}}, env.kd.context()); }, w.value(4, QStringLiteral("3000")).toInt());
            check(env, ok, QStringLiteral("context %1 = %2 (now %3)").arg(path, w.value(3), toJson(valueAtPath(env.kd.context(), path))));
        } else if (cmd == QLatin1String("print") && w.value(1) == QLatin1String("ctx")) {
            settle = false;
            const QVariant v = w.size() > 2 ? valueAtPath(env.kd.context(), w.value(2)) : QVariant(env.kd.context());
            say(QStringLiteral("  ctx %1 = %2").arg(w.value(2, QStringLiteral("(all)")), toJson(v)));
        } else if (cmd == QLatin1String("expect") && w.value(1) == QLatin1String("refused")) {
            settle = false;
            const QString code = w.value(2);
            const bool ok = waitUntil([&] { return std::any_of(env.refusals.cbegin(), env.refusals.cend(), [&](const auto &r) { return r.second == code; }); },
                                      w.value(3, QStringLiteral("2000")).toInt());
            check(env, ok, QStringLiteral("a refusal with %1").arg(code));
            env.refusals.clear();
        } else if (cmd == QLatin1String("expect") && w.value(1) == QLatin1String("no-refusal")) {
            settle = false;
            QStringList seen;
            for (const auto &r : std::as_const(env.refusals)) {
                seen << r.first + QLatin1Char(':') + r.second;
            }
            check(env, env.refusals.isEmpty(), QStringLiteral("no refusal (%1)").arg(seen.isEmpty() ? QStringLiteral("none") : seen.join(QStringLiteral(", "))));
            env.refusals.clear();
        } else if (cmd == QLatin1String("expect") && w.value(1) == QLatin1String("no-keys")) {
            settle = false;
            check(env, env.keys.taps.isEmpty(), QStringLiteral("no keyboard fallback (taps: %1)").arg(env.keys.taps.isEmpty() ? QStringLiteral("none") : env.keys.taps.join(QLatin1Char(' '))));
        } else if (cmd == QLatin1String("expect") && w.value(1) == QLatin1String("keys")) {
            settle = false;
            const QStringList want = w.mid(2);
            waitUntil([&] { return env.keys.taps.size() >= want.size(); }, 2000);
            check(env, env.keys.taps == want, QStringLiteral("recorded (never emitted) keys %1, wanted %2").arg(env.keys.taps.join(QLatin1Char(' ')), want.join(QLatin1Char(' '))));
        } else if (cmd == QLatin1String("expect") && w.value(1) == QLatin1String("notice")) {
            settle = false;
            const QString text = l.section(QLatin1Char(' '), 2);
            const bool ok = waitUntil([&] { return std::any_of(env.notices.cbegin(), env.notices.cend(), [&](const QString &n) { return n.contains(text); }); }, 2000);
            check(env, ok, QStringLiteral("a notice%1").arg(text.isEmpty() ? QString() : QStringLiteral(" containing \"%1\"").arg(text)));
            env.notices.clear();
        } else if (cmd == QLatin1String("expect") && w.value(1) == QLatin1String("no-notice")) {
            settle = false;
            check(env, env.notices.isEmpty(), QStringLiteral("no notice (%1)").arg(env.notices.isEmpty() ? QStringLiteral("none") : env.notices.join(QStringLiteral("; "))));
        } else if (cmd == QLatin1String("refusals") && w.value(1) == QLatin1String("clear")) {
            settle = false;
            env.refusals.clear();
        } else if (cmd == QLatin1String("wait")) {
            QEventLoop loop;
            QTimer::singleShot(rest.toInt(), &loop, &QEventLoop::quit);
            loop.exec();
        } else if (cmd.startsWith(QLatin1String("key"))) {
            env.engine.handle(PadEvent{cmd, PadEvent::KeyDown, 0, 0});
        } else if (cmd.startsWith(QLatin1String("knob"))) {
            if (rest == QLatin1String("press") || rest == QLatin1String("hold")) {
                env.engine.handle(PadEvent{cmd, PadEvent::PressDown, 0, 0});
            }
            if (rest == QLatin1String("press") || rest == QLatin1String("release")) {
                env.engine.handle(PadEvent{cmd, PadEvent::PressUp, 0, 0});
            }
            if (rest != QLatin1String("press") && rest != QLatin1String("hold") && rest != QLatin1String("release")) {
                const int n = rest.toInt();
                for (int i = 0; i < std::abs(n); ++i) {
                    env.engine.handle(PadEvent{cmd, PadEvent::Turn, n > 0 ? 1 : -1, 0});
                }
            }
        } else {
            std::fprintf(stderr, "line %d: unknown command '%s'\n", line, qPrintable(cmd));
            return 2;
        }
        if (!settle) {
            continue;
        }
        // let coalescers, acks and tap pacing run
        QEventLoop loop;
        QTimer::singleShot(qMax(30, env.engine.config().settings.coalesceMs * 3), &loop, &QEventLoop::quit);
        loop.exec();
        while (env.engine.pendingTaps() > 0) {
            QTimer::singleShot(10, &loop, &QEventLoop::quit);
            loop.exec();
        }
    }
    if (env.failures > 0) {
        say(QStringLiteral("%1 expectation(s) failed").arg(env.failures));
    }
    return env.failures > 0 ? 1 : 0;
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
        "Commands: run (default) | simulate [FILE|-] | verify | list-devices | list-capabilities | check-config | example-config | bench-dbus [N]"));
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
    QCommandLineOption traceOpt(QStringLiteral("trace"), QStringLiteral("log every Kdenlive call, reply, ack and epoch change"));
    QCommandLineOption jsonOpt(QStringLiteral("json"), QStringLiteral("list-capabilities: machine-readable output"));
    p.addOptions({configOpt, dryOpt, noGrabOpt, backendOpt, serviceOpt, writeOpt, noWriteOpt, forceWindowOpt, quietOpt, traceOpt, jsonOpt});
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
            say(QStringLiteral("  profile %1: %2 layers, %3 base bindings%4%5")
                    .arg(pr.name)
                    .arg(pr.layers.size())
                    .arg(pr.bindings.size())
                    .arg(pr.kdenlive ? QStringLiteral(", kdenlive") : QString(), pr.keyFallback ? QStringLiteral(", keyFallback") : QString()));
        }
        for (const QString &w : std::as_const(cfg->warnings)) {
            say(QStringLiteral("warning: %1").arg(w));
        }
        return 0;
    }
    if (cmd == QLatin1String("list-capabilities")) {
        // Read-only: Capabilities, ListActions and GetContext; no lease is taken.
        QDBusConnection bus = QDBusConnection::sessionBus();
        const QStringList services = p.isSet(serviceOpt) ? QStringList{p.value(serviceOpt)} : discoverKdenliveServices(bus);
        QJsonArray all;
        bool anyAvailable = false;
        for (const QString &s : services) {
            const CapabilityReport r = queryKdenlive(bus, s);
            anyAvailable = anyAvailable || r.status == CapabilityReport::Status::Available;
            if (p.isSet(jsonOpt)) {
                all.append(reportJson(r, &*cfg));
            } else {
                say(formatReport(r, &*cfg));
            }
        }
        if (p.isSet(jsonOpt)) {
            const QByteArray j = QJsonDocument(QJsonObject{{QStringLiteral("kdenlive"), all}}).toJson(QJsonDocument::Indented);
            std::fwrite(j.constData(), 1, size_t(j.size()), stdout);
        } else if (services.isEmpty()) {
            say(QStringLiteral("no running Kdenlive on the session bus (looked for org.kde.kdenlive-<pid>; use --kdenlive-service NAME)"));
        }
        return anyAvailable ? 0 : 3;
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
        // No pad and no virtual keyboard: keys are only recorded and printed.
        // With --kdenlive-service the real client talks to that Kdenlive.
        RecordingKeySink keys(true);
        std::unique_ptr<FakeKdenliveClient> fake;
        std::unique_ptr<KdenliveDBusClient> real;
        KdenliveClient *kd = nullptr;
        if (p.isSet(serviceOpt)) {
            real = std::make_unique<KdenliveDBusClient>(QDBusConnection::sessionBus());
            real->setServiceOverride(p.value(serviceOpt));
            real->setTrace(p.isSet(traceOpt));
            QObject::connect(real.get(), &KdenliveClient::message, [](const QString &m) { say(QStringLiteral("  [kdenlive] %1").arg(m)); });
            kd = real.get();
        } else {
            fake = std::make_unique<FakeKdenliveClient>(true);
            kd = fake.get();
        }
        StaticWindowTracker tracker;
        Engine engine(&keys, kd);
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
        SimEnv env{engine, *kd, fake.get(), tracker, keys, {}, {}, 0};
        QObject::connect(&engine, &Engine::notice, [&env](const QString &n) {
            say(QStringLiteral("  NOTICE: %1").arg(n));
            env.notices << n;
        });
        QObject::connect(kd, &KdenliveClient::refused, [&env](const QString &what, const QString &code, const QString &) { env.refusals.append({what, code}); });
        const int rc = simulate(env, in);
        if (real) {
            real->attachToPid(0);  // Unsubscribe: release the lease
            QEventLoop loop;
            QTimer::singleShot(200, &loop, &QEventLoop::quit);
            loop.exec();
        }
        return rc;
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
        QObject::connect(&client, &KdenliveClient::stateChanged, &loop, [&loop](KdenliveClient::State s) {
            if (s == KdenliveClient::State::Available || s == KdenliveClient::State::Absent) {
                loop.quit();
            }
        });
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
            client.control(QStringLiteral("bench"), QStringLiteral("playhead.jog"), (i % 2) ? -1 : 1, {});
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
        QTimer::singleShot(100, &loop, &QEventLoop::quit);  // trailing limited context signal
        loop.exec();
        const auto timing = client.contextTiming();
        say(QStringLiteral("context signals %1 (%2 with emittedAtMs); min spacing: emitted %3 ms, arrival %4 ms (arrival can bunch)")
                .arg(timing.received)
                .arg(timing.stamped)
                .arg(timing.minEmitGapMs)
                .arg(timing.minArrivalGapMs));
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
    QObject::connect(&engine, &Engine::notice, [dry, quiet](const QString &text) {
        if (quiet) {
            say(QStringLiteral("notice: %1").arg(text));  // otherwise logged with Engine::message
        }
        if (!dry) {
            desktopNotify(text);
        }
    });
    for (const QString &w : std::as_const(cfg->warnings)) {
        say(QStringLiteral("config warning: %1").arg(w));
    }
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
    if (dry) {
        QObject::connect(&dev, &PadDevice::padEvent, &engine, [](const PadEvent &e) { say(e.describe()); });
        QObject::connect(&engine, &Engine::dispatched, [](const QString &slot, const QString &binding, const QString &layer) {
            say(QStringLiteral("  %1 -> %2%3").arg(slot, binding, layer.isEmpty() ? QString() : QStringLiteral(" [layer %1]").arg(layer)));
        });
    }
    QObject::connect(&dev, &PadDevice::padEvent, &engine, &Engine::handle);
    // Hot reload: a valid edit replaces the config (pending knob motion is
    // dropped); an invalid one is reported and the running config stays.
    ConfigWatcher watcher(p.value(configOpt));
    watcher.setExtraFiles({defaultHardwareMapPath()});
    const DeviceMatch startedWith = cfg->device;
    QObject::connect(&watcher, &ConfigWatcher::reloaded, &engine, [&](const Config &c) {
        engine.setConfig(c);
        dev.setHardwareMap(c.hardware);
        say(QStringLiteral("config reloaded from %1 (%2 profiles, hardware map %3)").arg(watcher.path()).arg(c.profiles.size()).arg(c.hardwareSource));
        for (const QString &w : c.warnings) {
            say(QStringLiteral("config warning: %1").arg(w));
        }
        if (c.device.vendor != startedWith.vendor || c.device.product != startedWith.product || c.device.serial != startedWith.serial) {
            say(QStringLiteral("config: device changes take effect after a restart"));
        }
    });
    QObject::connect(&watcher, &ConfigWatcher::failed, [dry](const QString &e) {
        std::fprintf(stderr, "config not reloaded: %s\n", qPrintable(e));
        if (!dry) {
            desktopNotify(QStringLiteral("Config not reloaded: %1").arg(e));
        }
    });
    watcher.start();
    dev.start();
    const int rc = app.exec();
    dev.stop();
    return rc;
}
