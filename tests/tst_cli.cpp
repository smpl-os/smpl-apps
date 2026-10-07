// SPDX-License-Identifier: GPL-2.0-or-later
// Offline CLI commands of control-surfaced, run as a process. None of them
// opens an input device; ctest runs this on a private D-Bus.
#include <QDir>
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
        // No daemon on this private bus: an offline report (sysfs is only read).
        const QString cfg = m_home.path() + QStringLiteral("/layout.jsonc");
        writeFile(cfg, R"({"layout":"generic-12k2e","profiles":[]})");
        Run r = run({QStringLiteral("status"), QStringLiteral("--json"), QStringLiteral("-c"), cfg}, m_home.path());
        QCOMPARE(r.code, 0);
        QJsonObject j = r.json();
        QCOMPARE(j.value(QStringLiteral("daemon")).toBool(), false);
        QCOMPARE(j.value(QStringLiteral("mode")).toString(), QStringLiteral("offline"));
        QVERIFY(j.value(QStringLiteral("device")).toObject().contains(QStringLiteral("present")));
        const QJsonObject layout = j.value(QStringLiteral("layout")).toObject();
        if (!j.value(QStringLiteral("device")).toObject().value(QStringLiteral("present")).toBool()) {
            QCOMPARE(layout.value(QStringLiteral("id")).toString(), QStringLiteral("generic-12k2e"));
            QCOMPARE(layout.value(QStringLiteral("source")).toString(), QStringLiteral("config"));
        }
        QCOMPARE(j.value(QStringLiteral("config")).toObject().value(QStringLiteral("exists")).toBool(), true);
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
};

QTEST_GUILESS_MAIN(TestCli)
#include "tst_cli.moc"
