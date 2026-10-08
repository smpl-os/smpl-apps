// SPDX-License-Identifier: GPL-2.0-or-later
// Offline CLI commands of control-surfaced, run as a process. None of them
// opens an input device; ctest runs this on a private D-Bus.
#include "ewwmock.h"

#include <QCryptographicHash>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDir>
#include <QFileInfo>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QSet>
#include <QTemporaryDir>
#include <QTest>

namespace {
struct Run {
    int code = -1;
    QByteArray out, err;
    QJsonObject json() const { return QJsonDocument::fromJson(out).object(); }
};

Run run(const QStringList &args, const QString &home)
{
    QProcess p;
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.insert(QStringLiteral("XDG_CONFIG_HOME"), home + QStringLiteral("/config"));
    env.insert(QStringLiteral("XDG_DATA_HOME"), home + QStringLiteral("/data"));
    p.setProcessEnvironment(env);
    p.start(QStringLiteral(CS_DAEMON_BINARY), args);
    p.waitForFinished(15000);
    return Run{p.exitCode(), p.readAllStandardOutput(), p.readAllStandardError()};
}

void writeFile(const QString &path, const QByteArray &data)
{
    QDir().mkpath(QFileInfo(path).absolutePath());
    QFile f(path);
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write(data);
}
} // namespace

class TestCli : public QObject
{
    Q_OBJECT
    QTemporaryDir m_home;

private Q_SLOTS:
    void checkConfigJson()
    {
        const QString good = m_home.path() + QStringLiteral("/good.jsonc");
        writeFile(good, R"({"profiles":[{"name":"g","bindings":{"key3":"F13","knob1.cw":{"control":"nope.x"}}}]})");
        Run r = run({QStringLiteral("check-config"), QStringLiteral("-c"), good, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        QJsonObject j = r.json();
        QCOMPARE(j.value(QStringLiteral("ok")).toBool(), true);
        QVERIFY(j.value(QStringLiteral("error")).isNull());
        QCOMPARE(j.value(QStringLiteral("warnings")).toArray().size(), 2);
        QCOMPARE(j.value(QStringLiteral("warningDetails")).toArray().first().toObject().value(QStringLiteral("slot")).toString(), QStringLiteral("knob1.cw"));
        QCOMPARE(j.value(QStringLiteral("source")).toString(), QStringLiteral("file"));

        const QString bad = m_home.path() + QStringLiteral("/bad.jsonc");
        writeFile(bad, R"({"profiles":[{"name":"Kdenlive edit","bindings":{"key3":{"bogus":1}}}]})");
        r = run({QStringLiteral("check-config"), QStringLiteral("-c"), bad, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 2);
        j = r.json();
        QCOMPARE(j.value(QStringLiteral("ok")).toBool(), false);
        const QJsonObject e = j.value(QStringLiteral("error")).toObject();
        QCOMPARE(e.value(QStringLiteral("profile")).toString(), QStringLiteral("Kdenlive edit"));
        QCOMPARE(e.value(QStringLiteral("slot")).toString(), QStringLiteral("key3"));
        QVERIFY(e.value(QStringLiteral("message")).toString().contains(QStringLiteral("binding object")));

        r = run({QStringLiteral("check-config"), QStringLiteral("-c"), m_home.path() + QStringLiteral("/none.jsonc"), QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 2);
        QVERIFY(r.json().value(QStringLiteral("error")).toObject().value(QStringLiteral("message")).toString().contains(QStringLiteral("not found")));

        // No -c and no user config: the built-in example is checked.
        r = run({QStringLiteral("check-config"), QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        QCOMPARE(r.json().value(QStringLiteral("source")).toString(), QStringLiteral("built-in"));
        QVERIFY(r.json().value(QStringLiteral("profiles")).toArray().size() >= 2);
    }

    void statusOffline()
    {
        // No daemon on this private bus: an offline report (sysfs is only
        // read). A fake /sys keeps it independent of what is plugged in here.
        const QString empty = m_home.path() + QStringLiteral("/sys-empty");
        QDir().mkpath(empty);
        const QString cfg = m_home.path() + QStringLiteral("/layout.jsonc");
        writeFile(cfg, R"({"layout":"generic-12k2e","profiles":[]})");
        Run r = run({QStringLiteral("status"), QStringLiteral("--json"), QStringLiteral("-c"), cfg, QStringLiteral("--sys-root"), empty}, m_home.path());
        QCOMPARE(r.code, 0);
        QJsonObject j = r.json();
        QCOMPARE(j.value(QStringLiteral("daemon")).toBool(), false);
        QCOMPARE(j.value(QStringLiteral("mode")).toString(), QStringLiteral("offline"));
        QCOMPARE(j.value(QStringLiteral("device")).toObject().value(QStringLiteral("present")).toBool(), false);
        QJsonObject layout = j.value(QStringLiteral("layout")).toObject();
        QCOMPARE(layout.value(QStringLiteral("id")).toString(), QStringLiteral("generic-12k2e"));
        QCOMPARE(layout.value(QStringLiteral("source")).toString(), QStringLiteral("config"));
        QVERIFY(layout.value(QStringLiteral("firmwareLayout")).isNull());
        QVERIFY(layout.value(QStringLiteral("matchesFirmware")).isNull());
        QCOMPARE(j.value(QStringLiteral("config")).toObject().value(QStringLiteral("exists")).toBool(), true);
        QVERIFY(j.value(QStringLiteral("config")).toObject().value(QStringLiteral("warnings")).toArray().isEmpty());

        // A pad with the control-surface firmware (descriptors name the 15+3 board).
        const QString sys = m_home.path() + QStringLiteral("/sys-pad");
        const QString dev = sys + QStringLiteral("/bus/usb/devices/1-4");
        const QList<QPair<const char *, QByteArray>> attrs{{"idVendor", "1189"}, {"idProduct", "8890"}, {"serial", "key153"}, {"manufacturer", "OpenMacroPad"},
                                                          {"product", "Control Surface 15+3"}, {"bcdDevice", "0200"}, {"busnum", "1"}, {"devnum", "9"},
                                                          {"bNumInterfaces", " 2"}};
        for (const auto &[name, value] : attrs) {
            writeFile(dev + QLatin1Char('/') + QLatin1String(name), value + '\n');
        }
        // The config's layout overrides the firmware's, and the mismatch is reported.
        r = run({QStringLiteral("status"), QStringLiteral("--json"), QStringLiteral("-c"), cfg, QStringLiteral("--sys-root"), sys}, m_home.path());
        QCOMPARE(r.code, 0);
        j = r.json();
        const QJsonObject device = j.value(QStringLiteral("device")).toObject();
        QCOMPARE(device.value(QStringLiteral("present")).toBool(), true);
        QCOMPARE(device.value(QStringLiteral("firmware")).toObject().value(QStringLiteral("version")).toString(), QStringLiteral("2.0"));
        QCOMPARE(device.value(QStringLiteral("firmware")).toObject().value(QStringLiteral("versionSource")).toString(), QStringLiteral("bcdDevice"));
        layout = j.value(QStringLiteral("layout")).toObject();
        QCOMPARE(layout.value(QStringLiteral("id")).toString(), QStringLiteral("generic-12k2e"));
        QCOMPARE(layout.value(QStringLiteral("source")).toString(), QStringLiteral("config"));
        QCOMPARE(layout.value(QStringLiteral("firmwareLayout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));
        QCOMPARE(layout.value(QStringLiteral("firmwareSlots")).toInt(), 24);
        QCOMPARE(layout.value(QStringLiteral("matchesFirmware")).toBool(), false);
        const QJsonArray warnings = j.value(QStringLiteral("config")).toObject().value(QStringLiteral("warnings")).toArray();
        QCOMPARE(warnings.size(), 1);
        QVERIFY(warnings.first().toString().startsWith(QLatin1String("layout: the config's generic-12k2e")));
        r = run({QStringLiteral("check-config"), QStringLiteral("--json"), QStringLiteral("-c"), cfg, QStringLiteral("--sys-root"), sys}, m_home.path());
        QCOMPARE(r.code, 0);  // a warning, not an error
        j = r.json();
        QCOMPARE(j.value(QStringLiteral("warnings")).toArray().size(), 1);
        QCOMPARE(j.value(QStringLiteral("warningDetails")).toArray().first().toObject().value(QStringLiteral("message")).toString(), warnings.first().toString());
        QCOMPARE(j.value(QStringLiteral("layout")).toObject().value(QStringLiteral("matchesFirmware")).toBool(), false);
        // Without an override the firmware's board is the layout.
        const QString plain = m_home.path() + QStringLiteral("/plain.jsonc");
        writeFile(plain, R"({"profiles":[]})");
        r = run({QStringLiteral("status"), QStringLiteral("--json"), QStringLiteral("-c"), plain, QStringLiteral("--sys-root"), sys}, m_home.path());
        layout = r.json().value(QStringLiteral("layout")).toObject();
        QCOMPARE(layout.value(QStringLiteral("source")).toString(), QStringLiteral("firmware"));
        QCOMPARE(layout.value(QStringLiteral("matchesFirmware")).toBool(), true);
        r = run({QStringLiteral("check-config"), QStringLiteral("--json"), QStringLiteral("-c"), plain, QStringLiteral("--sys-root"), sys}, m_home.path());
        QVERIFY(r.json().value(QStringLiteral("warnings")).toArray().isEmpty());
        // Nothing plugged in and nothing set: the default.
        r = run({QStringLiteral("status"), QStringLiteral("--json"), QStringLiteral("-c"), plain, QStringLiteral("--sys-root"), empty}, m_home.path());
        QCOMPARE(r.json().value(QStringLiteral("layout")).toObject().value(QStringLiteral("source")).toString(), QStringLiteral("default"));
        r = run({QStringLiteral("status")}, m_home.path());
        QCOMPARE(r.code, 0);
        QVERIFY(r.out.contains("layout:"));
    }

    void brokenConfigWaitsInsteadOfExiting()
    {
        // run with a broken config must not exit (Restart=on-failure would
        // loop); it waits without touching the pad and stops cleanly on SIGTERM.
        const QString cfg = m_home.path() + QStringLiteral("/broken.jsonc");
        writeFile(cfg, "{ this is not json");
        QProcess p;
        QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
        env.insert(QStringLiteral("XDG_CONFIG_HOME"), m_home.path() + QStringLiteral("/config"));
        p.setProcessEnvironment(env);
        p.start(QStringLiteral(CS_DAEMON_BINARY), {QStringLiteral("run"), QStringLiteral("--dry-run"), QStringLiteral("--no-settings-api"),
                                                  QStringLiteral("--window-backend"), QStringLiteral("none"), QStringLiteral("-c"), cfg});
        QVERIFY(p.waitForStarted());
        QByteArray out;
        QTRY_VERIFY_WITH_TIMEOUT((out += p.readAllStandardOutput()).contains("waiting for a valid"), 5000);
        QTest::qWait(500);
        QCOMPARE(p.state(), QProcess::Running);
        p.terminate();
        QVERIFY(p.waitForFinished(5000));
        QCOMPARE(p.exitStatus(), QProcess::NormalExit);
        QCOMPARE(p.exitCode(), 0);
    }

    void listActionsOffline()
    {
        Run r = run({QStringLiteral("list-actions"), QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        const QJsonObject j = r.json();
        const QJsonArray actions = j.value(QStringLiteral("actions")).toArray();
        QCOMPARE(actions.size(), 71);
        int editing = 0, playback = 0;
        QSet<QString> ids;
        for (const auto &a : actions) {
            const QJsonObject o = a.toObject();
            ids.insert(o.value(QStringLiteral("id")).toString());
            editing += o.value(QStringLiteral("editing")).toBool();
            playback += o.value(QStringLiteral("playback")).toBool();
            QVERIFY(!o.value(QStringLiteral("group")).toString().isEmpty());
        }
        QCOMPARE(ids.size(), 71);
        QCOMPARE(editing, 30);
        QCOMPARE(playback, 7);
        QVERIFY(ids.contains(QStringLiteral("razor_tool")));
        QCOMPARE(j.value(QStringLiteral("controls")).toArray().size(), 10);
        QCOMPARE(j.value(QStringLiteral("commands")).toArray().size(), 3);
        QCOMPARE(j.value(QStringLiteral("contract")).toObject().value(QStringLiteral("interface")).toString(), QStringLiteral("org.kde.kdenlive.ControlSurface1"));
        r = run({QStringLiteral("list-actions")}, m_home.path());
        QCOMPARE(r.code, 0);
        QVERIFY(r.out.contains("[editing]"));
        QVERIFY(r.out.contains("playhead.jog"));
    }

    void firmwareCommandsNeedTheOpenFirmware()
    {
        // A fake, empty /sys: no device is ever opened by these checks.
        const QString sys = m_home.path() + QStringLiteral("/fake-sys");
        QDir().mkpath(sys);
        Run r = run({QStringLiteral("firmware-info"), QStringLiteral("--json"), QStringLiteral("--sys-root"), sys}, m_home.path());
        QCOMPARE(r.code, 1);
        QCOMPARE(r.json().value(QStringLiteral("ok")).toBool(), false);
        QVERIFY(r.json().value(QStringLiteral("error")).toString().contains(QStringLiteral("protocol v3")));
        r = run({QStringLiteral("enter-bootloader"), QStringLiteral("--json"), QStringLiteral("--yes"), QStringLiteral("--sys-root"), sys}, m_home.path());
        QCOMPARE(r.code, 1);
        QCOMPARE(r.json().value(QStringLiteral("ok")).toBool(), false);
    }

    void releaseMetadataMatchesTheImage()
    {
        const QString dir = QStringLiteral(CS_SOURCE_DIR "/firmware/release");
        const QStringList bins = QDir(dir).entryList({QStringLiteral("*.bin")}, QDir::Files);
        QVERIFY(!bins.isEmpty());
        QVERIFY(QFile::exists(dir + QStringLiteral("/LICENSE")));
        for (const QString &b : bins) {
            QFile img(dir + QLatin1Char('/') + b);
            QVERIFY(img.open(QIODevice::ReadOnly));
            const QByteArray data = img.readAll();
            QFile meta(dir + QLatin1Char('/') + QFileInfo(b).completeBaseName() + QStringLiteral(".json"));
            QVERIFY2(meta.open(QIODevice::ReadOnly), qPrintable(meta.fileName()));
            const QJsonObject m = QJsonDocument::fromJson(meta.readAll()).object();
            for (const char *k : {"name", "version", "board", "license", "sha256"}) {
                QVERIFY2(!m.value(QLatin1String(k)).toString().isEmpty(), k);
            }
            QCOMPARE(m.value(QStringLiteral("name")).toString() + QStringLiteral(".bin"), b);
            QCOMPARE(m.value(QStringLiteral("sha256")).toString(), QString::fromLatin1(QCryptographicHash::hash(data, QCryptographicHash::Sha256).toHex()));
            QCOMPARE(m.value(QStringLiteral("size")).toInt(), data.size());
            QVERIFY(data.size() <= 14336);
            QCOMPARE(m.value(QStringLiteral("license")).toString(), QStringLiteral("CC-BY-SA-3.0"));
        }
    }

    void featuresJson()
    {
        Run r = run({QStringLiteral("features"), QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        const QJsonObject j = r.json();
        QStringList kinds;
        for (const auto &k : j.value(QStringLiteral("bindingKinds")).toArray()) {
            kinds << k.toObject().value(QStringLiteral("kind")).toString();
        }
        QCOMPARE(kinds, (QStringList{QStringLiteral("keys"), QStringLiteral("mouse"), QStringLiteral("action"), QStringLiteral("control"), QStringLiteral("request"),
                                     QStringLiteral("cycle"), QStringLiteral("command"), QStringLiteral("cheatsheet"), QStringLiteral("none")}));
        const QJsonObject sheet = j.value(QStringLiteral("cheatsheet")).toObject();
        QCOMPARE(sheet.value(QStringLiteral("modes")).toArray().size(), 2);
        QVERIFY(sheet.value(QStringLiteral("options")).toObject().value(QStringLiteral("position")).toArray().contains(QStringLiteral("top-right")));
        QCOMPARE(sheet.value(QStringLiteral("defaults")).toObject(),
                 (QJsonObject{{QStringLiteral("opacity"), 0.35}, {QStringLiteral("autoHideMs"), 8000}, {QStringLiteral("position"), QStringLiteral("center")}}));
        QVERIFY(sheet.value(QStringLiteral("options")).toObject().value(QStringLiteral("opacity")).toString().contains(QStringLiteral("default 0.35")));
        // The overlay's icon contract.
        const QJsonObject icons = sheet.value(QStringLiteral("icons")).toObject();
        QCOMPARE(icons.keys(), (QStringList{QStringLiteral("auto"), QStringLiteral("set"), QStringLiteral("version")}));
        QCOMPARE(icons.value(QStringLiteral("set")).toString(), QStringLiteral("tabler-outline"));
        QCOMPARE(icons.value(QStringLiteral("version")).toString(), QStringLiteral("3.49.0"));
        const QJsonArray autoIcons = icons.value(QStringLiteral("auto")).toArray();
        QVERIFY(autoIcons.size() > 60);
        for (const char *n : {"help-circle", "player-play", "volume-2", "brand-github", "chart-dots-3", "terminal", "brackets-contain-start", "color-filter", "stack-2", "rotate"}) {
            QVERIFY2(autoIcons.contains(QLatin1String(n)), n);
        }
        const QJsonObject fields = j.value(QStringLiteral("bindingFields")).toObject();
        QVERIFY(fields.contains(QStringLiteral("icon")) && fields.contains(QStringLiteral("label")) && fields.contains(QStringLiteral("ifInstalled")));
        const QJsonArray keys = j.value(QStringLiteral("keyNames")).toArray();
        QVERIFY(keys.contains(QStringLiteral("F24")));
        QVERIFY(keys.contains(QStringLiteral("PLAYPAUSE")));
        QVERIFY(!keys.contains(QStringLiteral("POWER")));  // never advertised
        QCOMPARE(j.value(QStringLiteral("mouseNames")).toArray().size(), 9);
        QCOMPARE(j.value(QStringLiteral("slots")).toObject().value(QStringLiteral("maxKeys")).toInt(), 16);
        QCOMPARE(j.value(QStringLiteral("slots")).toObject().value(QStringLiteral("maxKnobs")).toInt(), 3);
        QCOMPARE(j.value(QStringLiteral("dbus")).toObject().value(QStringLiteral("interface")).toString(), QStringLiteral("org.smplos.ControlSurface1"));
        QVERIFY(j.value(QStringLiteral("layouts")).toObject().value(QStringLiteral("builtin")).toArray().size() >= 5);
        // Simple options and the input-mode names a settings UI shows.
        const QJsonObject device = j.value(QStringLiteral("device")).toObject();
        QCOMPARE(device.value(QStringLiteral("inputDefault")).toString(), QStringLiteral("auto"));
        QCOMPARE(device.value(QStringLiteral("inputLabels")).toObject(),
                 (QJsonObject{{QStringLiteral("auto"), QStringLiteral("Automatic")}, {QStringLiteral("evdev"), QStringLiteral("Keymap (compatible)")},
                              {QStringLiteral("raw"), QStringLiteral("Raw (fastest)")}}));
        QStringList optionKeys;
        for (const auto &o : j.value(QStringLiteral("options")).toArray()) {
            optionKeys << o.toObject().value(QStringLiteral("key")).toString();
        }
        QVERIFY(optionKeys.contains(QStringLiteral("input")) && optionKeys.contains(QStringLiteral("cheatsheet.opacity")));
        QCOMPARE(j.value(QStringLiteral("slots")).toObject().value(QStringLiteral("shiftSupported")), QJsonValue(false));
        // (tst_config checks that every advertised name and example parses.)
    }

    void setAndGet()
    {
        QFile ex(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"));
        QVERIFY(ex.open(QIODevice::ReadOnly));
        const QByteArray example = ex.readAll();
        const QString cfg = m_home.path() + QStringLiteral("/set/config.jsonc");
        writeFile(cfg, example);

        Run r = run({QStringLiteral("set"), QStringLiteral("input"), QStringLiteral("keymap"), QStringLiteral("-c"), cfg, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        QJsonObject j = r.json();
        QVERIFY(j.value(QStringLiteral("ok")).toBool());
        QVERIFY(j.value(QStringLiteral("changed")).toBool());
        QCOMPARE(j.value(QStringLiteral("old")).toString(), QStringLiteral("auto"));
        QCOMPARE(j.value(QStringLiteral("new")).toString(), QStringLiteral("evdev"));
        QCOMPARE(j.value(QStringLiteral("backup")).toString(), cfg + QStringLiteral(".bak"));
        QFile bak(cfg + QStringLiteral(".bak"));
        QVERIFY(bak.open(QIODevice::ReadOnly));
        QCOMPARE(bak.readAll(), example);
        QFile now(cfg);
        QVERIFY(now.open(QIODevice::ReadOnly));
        QCOMPARE(now.readAll(), QByteArray(example).replace("\"input\": \"auto\"", "\"input\": \"evdev\""));

        r = run({QStringLiteral("set"), QStringLiteral("input"), QStringLiteral("evdev"), QStringLiteral("-c"), cfg}, m_home.path());
        QCOMPARE(r.code, 0);
        QVERIFY(r.out.contains("already"));
        r = run({QStringLiteral("set"), QStringLiteral("input"), QStringLiteral("fast"), QStringLiteral("-c"), cfg, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 2);
        QCOMPARE(r.json().value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(), QStringLiteral("invalid-value"));
        r = run({QStringLiteral("set"), QStringLiteral("nope"), QStringLiteral("1"), QStringLiteral("-c"), cfg}, m_home.path());
        QCOMPARE(r.code, 2);
        QVERIFY(r.err.contains("no option 'nope'"));
        r = run({QStringLiteral("set"), QStringLiteral("input"), QStringLiteral("-c"), cfg}, m_home.path());  // no value: how to
        QCOMPARE(r.code, 2);
        QVERIFY2(r.err.contains("usage: control-surfaced set input auto|evdev|raw   (now: \"evdev\")"), r.err.constData());

        r = run({QStringLiteral("get"), QStringLiteral("input"), QStringLiteral("-c"), cfg, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        QCOMPARE(r.json().value(QStringLiteral("value")).toString(), QStringLiteral("evdev"));
        QCOMPARE(r.json().value(QStringLiteral("default")).toString(), QStringLiteral("auto"));
        r = run({QStringLiteral("get"), QStringLiteral("-c"), cfg, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        QCOMPARE(r.json().value(QStringLiteral("options")).toArray().size(), 9);
        r = run({QStringLiteral("get"), QStringLiteral("-c"), cfg}, m_home.path());
        QVERIFY(r.out.contains("cheatsheet.autoHideMs = 8000  (default)"));
        QVERIFY(r.out.contains("control-surfaced set KEY VALUE"));
        r = run({QStringLiteral("get"), QStringLiteral("nope"), QStringLiteral("-c"), cfg}, m_home.path());
        QCOMPARE(r.code, 2);

        // No file yet: it starts from the shipped example.
        const QString fresh = m_home.path() + QStringLiteral("/set/fresh/config.jsonc");
        r = run({QStringLiteral("set"), QStringLiteral("cheatsheet.opacity"), QStringLiteral("0.5"), QStringLiteral("-c"), fresh, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        QFile f(fresh);
        QVERIFY(f.open(QIODevice::ReadOnly));
        QCOMPARE(f.readAll(), QByteArray(example).replace("\"opacity\": 0.35", "\"opacity\": 0.5"));

        // A file that cannot be written: exit 3, nothing changed.
        const QString lockedDir = m_home.path() + QStringLiteral("/set/locked");
        writeFile(lockedDir + QStringLiteral("/config.jsonc"), example);
        QVERIFY(QFile::setPermissions(lockedDir, QFileDevice::ReadOwner | QFileDevice::ExeOwner));
        r = run({QStringLiteral("set"), QStringLiteral("input"), QStringLiteral("raw"), QStringLiteral("-c"), lockedDir + QStringLiteral("/config.jsonc")}, m_home.path());
        QFile::setPermissions(lockedDir, QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
        QCOMPARE(r.code, 3);
    }

    // `set` waits for a running daemon to load the change and says so.
    void setAppliesToRunningDaemon()
    {
        if (qEnvironmentVariable("CS_PRIVATE_BUS") != QLatin1String("1")) {
            QSKIP("starts a daemon with the settings API: private bus only (ctest)");
        }
        const QString cfg = m_home.path() + QStringLiteral("/applied.jsonc");
        writeFile(cfg, R"({"device": {"serial": "cs-test-no-such-pad", "input": "auto"}, "profiles": [{"name": "global", "bindings": {"key1": "ctrl+t"}}]})");
        QProcess p;
        QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
        env.insert(QStringLiteral("XDG_CONFIG_HOME"), m_home.path() + QStringLiteral("/config"));
        p.setProcessEnvironment(env);
        p.start(QStringLiteral(CS_DAEMON_BINARY), {QStringLiteral("run"), QStringLiteral("--quiet"), QStringLiteral("--dry-run"), QStringLiteral("--window-backend"),
                                                  QStringLiteral("none"), QStringLiteral("-c"), cfg});
        QVERIFY(p.waitForStarted());
        QTRY_VERIFY_WITH_TIMEOUT(QDBusConnection::sessionBus().interface()->isServiceRegistered(QStringLiteral("org.smplos.ControlSurface")), 5000);

        Run r = run({QStringLiteral("set"), QStringLiteral("input"), QStringLiteral("evdev"), QStringLiteral("-c"), cfg, QStringLiteral("--json")}, m_home.path());
        QCOMPARE(r.code, 0);
        QJsonObject d = r.json().value(QStringLiteral("daemon")).toObject();
        QVERIFY2(d.value(QStringLiteral("applied")).toBool(), r.out.constData());
        QCOMPARE(d.value(QStringLiteral("inputMode")).toString(), QStringLiteral("evdev-chords"));
        r = run({QStringLiteral("set"), QStringLiteral("cheatsheet.position"), QStringLiteral("top"), QStringLiteral("-c"), cfg}, m_home.path());
        QCOMPARE(r.code, 0);
        QVERIFY2(r.out.contains("the running daemon applied it"), r.out.constData());
        // Another file than the daemon's: said so, not waited for.
        const QString other = m_home.path() + QStringLiteral("/other.jsonc");
        writeFile(other, R"({"profiles": []})");
        r = run({QStringLiteral("set"), QStringLiteral("input"), QStringLiteral("raw"), QStringLiteral("-c"), other}, m_home.path());
        QCOMPARE(r.code, 0);
        QVERIFY2(r.out.contains("uses another config file"), r.out.constData());

        p.terminate();
        QVERIFY(p.waitForFinished(8000));
    }

    void cheatsheetCommand()
    {
        const QString cfg = m_home.path() + QStringLiteral("/sheet.jsonc");
        writeFile(cfg, R"({"profiles":[{"name":"Brave","match":{"class":"^brave"},"bindings":{"key1":"ctrl+t"}},
                                       {"name":"global","bindings":{"key15":{"cheatsheet":"toggle"}}}]})");
        Run r = run({QStringLiteral("cheatsheet"), QStringLiteral("--json"), QStringLiteral("-c"), cfg, QStringLiteral("--window"), QStringLiteral("brave-browser")}, m_home.path());
        QCOMPARE(r.code, 0);
        QJsonObject j = r.json();
        QCOMPARE(j.value(QStringLiteral("profile")).toString(), QStringLiteral("Brave"));
        QCOMPARE(j.value(QStringLiteral("keys")).toArray().at(0).toObject().value(QStringLiteral("label")).toString(), QStringLiteral("Ctrl+T"));
        QCOMPARE(j.value(QStringLiteral("keys")).toArray().at(14).toObject().value(QStringLiteral("label")).toString(), QStringLiteral("Cheatsheet"));
        r = run({QStringLiteral("cheatsheet"), QStringLiteral("-c"), cfg, QStringLiteral("--window"), QStringLiteral("brave-browser")}, m_home.path());
        QCOMPARE(r.code, 0);
        QVERIFY(r.out.contains("Ctrl+T"));
        r = run({QStringLiteral("cheatsheet"), QStringLiteral("-c"), cfg, QStringLiteral("--window"), QStringLiteral("x"), QStringLiteral("--context"), QStringLiteral("[")}, m_home.path());
        QCOMPARE(r.code, 2);
        // No daemon on this private bus.
        r = run({QStringLiteral("cheatsheet"), QStringLiteral("--follow"), QStringLiteral("-c"), cfg}, m_home.path());
        QCOMPARE(r.code, 3);
    }

    void ewwPushFromDaemon()
    {
        // The real daemon (dry run, a serial no pad has, so nothing is opened)
        // with smplOS's unit flags, a recording eww, and the settings API on
        // this private bus.
        if (qEnvironmentVariable("CS_PRIVATE_BUS") != QLatin1String("1")) {
            QSKIP("starts a daemon with the settings API: private bus only (ctest)");
        }
        QTemporaryDir bin;
        QVERIFY(ewwmock::install(bin.path()));
        const QString log = bin.path() + QStringLiteral("/calls.log");
        const QString ewwDir = m_home.path() + QStringLiteral("/eww-config");
        auto daemon = [&](const QByteArray &cheatsheet, QProcess &p) {
            const QString cfg = m_home.path() + QStringLiteral("/eww-daemon.jsonc");
            writeFile(cfg, R"({"device": {"serial": "cs-test-no-such-pad"}, "cheatsheet": )" + cheatsheet +
                               R"(, "profiles": [{"name": "global", "bindings": {"key1": "ctrl+t", "key15": {"cheatsheet": "toggle"}}}]})");
            QFile::remove(log);
            QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
            env.insert(QStringLiteral("XDG_CONFIG_HOME"), m_home.path() + QStringLiteral("/config"));
            env.insert(QStringLiteral("PATH"), bin.path() + QLatin1Char(':') + env.value(QStringLiteral("PATH")));
            env.insert(QStringLiteral("EWW_MOCK_LOG"), log);
            p.setProcessEnvironment(env);
            p.start(QStringLiteral(CS_DAEMON_BINARY), {QStringLiteral("run"), QStringLiteral("--quiet"), QStringLiteral("--dry-run"),
                                                      QStringLiteral("--window-backend"), QStringLiteral("none"), QStringLiteral("-c"), cfg,
                                                      QStringLiteral("--eww-window"), QStringLiteral("pad-cheatsheet"), QStringLiteral("--eww-config"), ewwDir});
            QVERIFY(p.waitForStarted());
            QDBusConnection bus = QDBusConnection::sessionBus();
            QTRY_VERIFY_WITH_TIMEOUT(bus.interface()->isServiceRegistered(QStringLiteral("org.smplos.ControlSurface")), 5000);
        };
        auto call = [](const char *method) {
            auto m = QDBusMessage::createMethodCall(QStringLiteral("org.smplos.ControlSurface"), QStringLiteral("/org/smplos/ControlSurface"),
                                                    QStringLiteral("org.smplos.ControlSurface1"), QLatin1String(method));
            return QDBusConnection::sessionBus().call(m, QDBus::Block, 3000);
        };
        auto stop = [](QProcess &p) {
            p.terminate();
            QVERIFY(p.waitForFinished(8000));
            QCOMPARE(p.exitStatus(), QProcess::NormalExit);
            QCOMPARE(p.exitCode(), 0);
        };

        {
            QProcess p;
            daemon(R"({"position": "top-right"})", p);
            QTRY_COMPARE_WITH_TIMEOUT(ewwmock::summaries(log), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}), 5000);
            QCOMPARE(call("ShowCheatsheet").type(), QDBusMessage::ReplyMessage);
            QTRY_COMPARE_WITH_TIMEOUT(ewwmock::summaries(log).mid(2), (QStringList{QStringLiteral("update pad_sheet visible=true"), QStringLiteral("open pad-cheatsheet @top right")}), 5000);
            for (const QStringList &c : ewwmock::calls(log)) {
                QCOMPARE(c.mid(0, 2), (QStringList{QStringLiteral("--config"), ewwDir}));
            }
            const QJsonObject st = QJsonDocument::fromJson(call("GetStatus").arguments().value(0).toString().toUtf8()).object();
            const QJsonObject sheet = st.value(QStringLiteral("cheatsheet")).toObject();
            QCOMPARE(sheet.value(QStringLiteral("visible")).toBool(), true);
            const QJsonObject eww = sheet.value(QStringLiteral("eww")).toObject();
            QCOMPARE(eww.value(QStringLiteral("enabled")).toBool(), true);
            QCOMPARE(eww.value(QStringLiteral("window")).toString(), QStringLiteral("pad-cheatsheet"));
            QCOMPARE(eww.value(QStringLiteral("variable")).toString(), QStringLiteral("pad_sheet"));
            QCOMPARE(eww.value(QStringLiteral("calls")).toInt(), 4);
            stop(p);
            // Hidden in eww before the daemon is gone.
            QCOMPARE(ewwmock::summaries(log).mid(4), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
        }
        {
            // The config overrides the unit's flags: off.
            QProcess p;
            daemon(R"({"eww": false})", p);
            QCOMPARE(call("ShowCheatsheet").type(), QDBusMessage::ReplyMessage);
            QTest::qWait(300);
            stop(p);
            QVERIFY(ewwmock::calls(log).isEmpty());
        }
        {
            // ... or another window; the unset eww config dir stays the unit's.
            QProcess p;
            daemon(R"({"eww": {"window": "my-sheet"}})", p);
            QCOMPARE(call("ShowCheatsheet").type(), QDBusMessage::ReplyMessage);
            QTRY_COMPARE_WITH_TIMEOUT(ewwmock::summaries(log).size(), 4, 5000);
            stop(p);
            QCOMPARE(ewwmock::summaries(log), (QStringList{QStringLiteral("close my-sheet"), QStringLiteral("update pad_sheet visible=false"),
                                                           QStringLiteral("update pad_sheet visible=true"), QStringLiteral("open my-sheet @center"),
                                                           QStringLiteral("close my-sheet"), QStringLiteral("update pad_sheet visible=false")}));
            QCOMPARE(ewwmock::calls(log).first().mid(0, 2), (QStringList{QStringLiteral("--config"), ewwDir}));
        }
    }
};

QTEST_GUILESS_MAIN(TestCli)
#include "tst_cli.moc"
