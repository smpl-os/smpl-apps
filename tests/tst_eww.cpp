// SPDX-License-Identifier: GPL-2.0-or-later
// The cheatsheet pushed into eww (EwwSink) against a recording stand-in eww:
// call order, anchors, reopen on a new position, coalescing, failures, exit,
// and how the config's "cheatsheet": {"eww": ...} sits over the CLI defaults.
#include "cheatsheet.h"
#include "config.h"
#include "engine.h"
#include "ewwmock.h"
#include "ewwsink.h"
#include "kdenliveclient.h"
#include "keysink.h"

#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

using namespace cs;

namespace {
const WindowInfo kBrave{QStringLiteral("brave-browser"), QStringLiteral("x"), 77, QStringLiteral("0x2")};
const WindowInfo kOther{QStringLiteral("foot"), QStringLiteral("y"), 78, QStringLiteral("0x3")};

std::optional<Config> config(const QByteArray &json, QString *err = nullptr)
{
    QString e;
    auto c = parseConfig(json, QString(), &e);
    if (err) {
        *err = e;
    }
    return c;
}

QByteArray sheetConfig(const char *position)
{
    return QByteArray(R"({"cheatsheet": {"position": ")") + position + R"("}, "profiles": [
        {"name": "Brave", "match": {"class": "^brave"}, "bindings": {"key1": "ctrl+t"}},
        {"name": "global", "bindings": {"key15": {"cheatsheet": "toggle"}, "key2": "ctrl+z"}}]})";
}

struct Rig {
    RecordingKeySink keys{false};
    FakeKdenliveClient kd;
    Engine engine{&keys, &kd};
    Cheatsheet sheet{&engine, &kd};
    EwwSink sink{&sheet};
    explicit Rig(const QByteArray &json)
    {
        engine.setConfig(*config(json));
        engine.setActiveWindow(kBrave);
    }
};
} // namespace

class TestEww : public QObject
{
    Q_OBJECT
    QTemporaryDir m_dir;
    QString m_bin, m_log;

    QStringList calls() const { return ewwmock::summaries(m_log); }
    void flag(const char *name, bool on)
    {
        const QString f = m_bin + QLatin1Char('/') + QLatin1String(name);
        if (on) {
            QFile file(f);
            QVERIFY(file.open(QIODevice::WriteOnly));
        } else {
            QFile::remove(f);
        }
    }
    EwwHook hook(const QString &window = QStringLiteral("pad-cheatsheet")) const
    {
        EwwHook h;
        h.enabled = true;
        h.window = window;
        h.configDir = m_dir.path() + QStringLiteral("/ewwcfg");
        return h;
    }

private Q_SLOTS:
    void initTestCase()
    {
        m_bin = m_dir.path() + QStringLiteral("/bin");
        QVERIFY(QDir().mkpath(m_bin));
        QVERIFY(ewwmock::install(m_bin));
        m_log = m_dir.path() + QStringLiteral("/calls.log");
        qputenv("EWW_MOCK_LOG", m_log.toUtf8());
        qputenv("PATH", (m_bin + QLatin1Char(':')).toUtf8() + qgetenv("PATH"));
    }

    void init()
    {
        QFile::remove(m_log);
        flag("slow", false);
        flag("fail-update", false);
    }

    void anchors()
    {
        const QList<QPair<QString, QString>> want{
            {QStringLiteral("center"), QStringLiteral("center")},          {QStringLiteral("top"), QStringLiteral("top center")},
            {QStringLiteral("bottom"), QStringLiteral("bottom center")},   {QStringLiteral("left"), QStringLiteral("center left")},
            {QStringLiteral("right"), QStringLiteral("center right")},     {QStringLiteral("top-left"), QStringLiteral("top left")},
            {QStringLiteral("top-right"), QStringLiteral("top right")},    {QStringLiteral("bottom-left"), QStringLiteral("bottom left")},
            {QStringLiteral("bottom-right"), QStringLiteral("bottom right")},
        };
        QCOMPARE(want.size(), CheatsheetOptions::positions().size());
        for (const auto &[position, anchor] : want) {
            QCOMPARE(EwwHook::anchorFor(position), anchor);
        }
        QCOMPARE(EwwHook::anchorFor(QStringLiteral("nowhere")), QStringLiteral("center"));
    }

    void configOverCliDefaults()
    {
        // smplOS's unit: run --eww-window pad-cheatsheet --eww-config DIR
        EwwHook cli;
        cli.enabled = true;
        cli.window = QStringLiteral("pad-cheatsheet");
        cli.configDir = QStringLiteral("/home/u/.config/eww");
        auto over = [&cli](const char *cheatsheet) {
            const auto c = config(QByteArray(R"({"cheatsheet": )") + cheatsheet + R"(, "profiles": []})");
            return c ? c->cheatsheet.eww.over(cli) : EwwHook{};
        };
        QCOMPARE(config(R"({"profiles": []})")->cheatsheet.eww.over(cli), cli);  // nothing set: the CLI's
        QCOMPARE(over(R"({"opacity": 0.5})"), cli);
        QCOMPARE(over(R"({"eww": true})"), cli);
        EwwHook h = over(R"({"eww": false})");
        QVERIFY(!h.enabled);  // the config wins over the unit
        h = over(R"({"eww": {"window": "my-sheet", "variable": "sheet2"}})");
        QVERIFY(h.enabled);
        QCOMPARE(h.window, QStringLiteral("my-sheet"));
        QCOMPARE(h.variable, QStringLiteral("sheet2"));
        QCOMPARE(h.configDir, cli.configDir);  // not set: kept
        QCOMPARE(h.binary, QStringLiteral("eww"));
        h = over(R"({"eww": {"window": ""}})");
        QVERIFY(h.enabled);
        QVERIFY(h.window.isEmpty());  // explicitly the variable only
        h = over(R"({"eww": {"enabled": false, "window": "x"}})");
        QVERIFY(!h.enabled);
        h = over(R"({"eww": {"config": "~/eww2", "binary": "/opt/eww/bin/eww"}})");
        QCOMPARE(h.configDir, QDir::homePath() + QStringLiteral("/eww2"));
        QCOMPARE(h.binary, QStringLiteral("/opt/eww/bin/eww"));
        QCOMPARE(h.window, cli.window);

        // Without CLI flags the push is off unless the config turns it on.
        const EwwHook none;
        QVERIFY(!config(R"({"profiles": []})")->cheatsheet.eww.over(none).enabled);
        h = config(R"({"cheatsheet": {"eww": true}, "profiles": []})")->cheatsheet.eww.over(none);
        QVERIFY(h.enabled);
        QCOMPARE(h.variable, QStringLiteral("pad_sheet"));
        QVERIFY(h.window.isEmpty());
        QVERIFY(h.configDir.isEmpty());

        for (const char *bad : {R"({"eww": 3})", R"({"eww": "yes"})", R"({"eww": {"bogus": 1}})", R"({"eww": {"variable": "a b"}})",
                                R"({"eww": {"window": "x;y"}})", R"({"eww": {"enabled": "yes"}})", R"({"eww": {"binary": ""}})",
                                R"({"eww": {"window": 5}})"}) {
            QString err;
            QVERIFY2(!config(QByteArray(R"({"cheatsheet": )") + bad + R"(, "profiles": []})", &err), bad);
            QVERIFY2(err.contains(QLatin1String("cheatsheet.eww")), qPrintable(err));
        }
    }

    void offIsSilent()
    {
        Rig r(sheetConfig("center"));
        r.sheet.show();
        r.sheet.hide();
        r.sink.setOptions(EwwHook{});  // explicitly off
        r.sheet.toggle();
        QTest::qWait(150);
        QVERIFY(calls().isEmpty());
        QVERIFY(!r.sink.isBusy());
    }

    void showChangeHide()
    {
        Rig r(sheetConfig("bottom"));
        const EwwHook h = hook();
        r.sink.setOptions(h);
        QTRY_VERIFY(!r.sink.isBusy());
        // At start: hidden, and a window a crash may have left is closed once.
        QCOMPARE(calls(), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));

        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(2), (QStringList{QStringLiteral("update pad_sheet visible=true"), QStringLiteral("open pad-cheatsheet @bottom center")}));
        const auto raw = ewwmock::calls(m_log);
        for (const QStringList &c : raw) {
            QCOMPARE(c.mid(0, 2), (QStringList{QStringLiteral("--config"), h.configDir}));
        }
        // The variable is the whole GetCheatsheet content.
        const QString arg = raw.at(2).at(3);
        QVERIFY(arg.startsWith(QLatin1String("pad_sheet=")));
        const QJsonObject sent = QJsonDocument::fromJson(arg.mid(10).toUtf8()).object();
        QCOMPARE(sent, r.sheet.content());
        QCOMPARE(sent.value(QStringLiteral("profile")).toString(), QStringLiteral("Brave"));

        // A change while shown: one update, the window stays.
        r.engine.setActiveWindow(kOther);
        QTRY_COMPARE(calls().size(), 5);
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().at(4), QStringLiteral("update pad_sheet visible=true"));
        QVERIFY(ewwmock::calls(m_log).at(4).at(3).contains(QLatin1String("\"profile\":\"global\"")));
        // Same content again: nothing sent.
        r.engine.setActiveWindow(kOther);
        r.sheet.invalidate();
        QTest::qWait(150);
        QCOMPARE(calls().size(), 5);

        r.sheet.hide();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(5), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
        r.sheet.hide();
        QTest::qWait(100);
        QCOMPARE(calls().size(), 7);
    }

    void positionChangeReopens()
    {
        Rig r(sheetConfig("bottom"));
        r.sink.setOptions(hook());
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().size(), 4);
        r.engine.setConfig(*config(sheetConfig("top-left")));  // e.g. Settings changed the position
        QTRY_COMPARE(calls().size(), 7);
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(4), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=true"),
                                              QStringLiteral("open pad-cheatsheet @top left")}));
        // While hidden a new position only takes effect at the next show.
        r.sheet.hide();
        QTRY_VERIFY(!r.sink.isBusy());
        r.engine.setConfig(*config(sheetConfig("right")));
        QTest::qWait(150);
        QCOMPARE(calls().size(), 9);
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(9), (QStringList{QStringLiteral("update pad_sheet visible=true"), QStringLiteral("open pad-cheatsheet @center right")}));
    }

    void variableOnly()
    {
        Rig r(sheetConfig("top"));
        EwwHook h;
        h.enabled = true;
        r.sink.setOptions(h);
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        r.sheet.hide();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls(), (QStringList{QStringLiteral("update pad_sheet visible=false"), QStringLiteral("update pad_sheet visible=true"),
                                       QStringLiteral("update pad_sheet visible=false")}));
        QCOMPARE(ewwmock::calls(m_log).first().first(), QStringLiteral("update"));  // no --config: eww's default
    }

    void coalescesWhileBusy()
    {
        flag("slow", true);  // 300 ms per call
        Rig r(sheetConfig("center"));
        r.sink.setOptions(hook(QString()));
        QVERIFY(r.sink.isBusy());
        for (int i = 0; i < 5; ++i) {
            r.sheet.toggle();  // ends shown
        }
        QVERIFY(r.sheet.isVisible());
        QTRY_VERIFY_WITH_TIMEOUT(!r.sink.isBusy(), 5000);
        // One call at a time, and only the latest state after the busy one.
        QCOMPARE(calls(), (QStringList{QStringLiteral("update pad_sheet visible=false"), QStringLiteral("update pad_sheet visible=true")}));
    }

    void failedUpdateSkipsOpen()
    {
        flag("fail-update", true);  // eww without a running daemon
        Rig r(sheetConfig("center"));
        QSignalSpy messages(&r.sink, &EwwSink::message);
        r.sink.setOptions(hook());
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QTest::qWait(150);
        // No `open`: it would start an eww daemon.
        QCOMPARE(calls(), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false"),
                                       QStringLiteral("update pad_sheet visible=true")}));
        QCOMPARE(messages.size(), 1);  // reported once
        QVERIFY(messages.first().first().toString().contains(QLatin1String("Failed to connect to daemon")));
        const QJsonObject st = r.sink.status();
        QCOMPARE(st.value(QStringLiteral("failures")).toInt(), 2);
        QCOMPARE(st.value(QStringLiteral("calls")).toInt(), 3);
        QVERIFY(st.value(QStringLiteral("lastError")).toString().startsWith(QLatin1String("update: Failed to connect")));

        flag("fail-update", false);  // eww is back; the next change shows it
        r.engine.setActiveWindow(kOther);
        QTRY_COMPARE(calls().size(), 5);
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(3), (QStringList{QStringLiteral("update pad_sheet visible=true"), QStringLiteral("open pad-cheatsheet @center")}));
        QCOMPARE(messages.size(), 2);
        QCOMPARE(messages.last().first().toString(), QStringLiteral("eww: working again"));
    }

    void missingBinary()
    {
        Rig r(sheetConfig("center"));
        QSignalSpy messages(&r.sink, &EwwSink::message);
        EwwHook h = hook(QString());
        h.binary = m_dir.path() + QStringLiteral("/no-such-eww");
        r.sink.setOptions(h);
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(messages.size(), 1);
        QVERIFY(messages.first().first().toString().contains(QLatin1String("cannot run")));
        QCOMPARE(r.sink.status().value(QStringLiteral("failures")).toInt(), 2);
    }

    void optionsChangeWhileShown()
    {
        Rig r(sheetConfig("center"));
        r.sink.setOptions(hook());
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().size(), 4);
        EwwHook moved = hook(QStringLiteral("sheet-b"));
        moved.variable = QStringLiteral("sheet_b");
        r.sink.setOptions(moved);  // old one taken down, new one shown
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(4), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false"),
                                              QStringLiteral("update sheet_b visible=true"), QStringLiteral("open sheet-b @center")}));
        r.sink.setOptions(EwwHook{});  // turned off while shown: hidden, then silent
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(8), (QStringList{QStringLiteral("close sheet-b"), QStringLiteral("update sheet_b visible=false")}));
        r.sheet.hide();
        r.sheet.show();
        QTest::qWait(150);
        QCOMPARE(calls().size(), 10);
    }

    void forceHideAlwaysClosesTheWindow()
    {
        // HideCheatsheet (a click on the overlay) when the daemon already
        // thinks it is hidden: eww is told again, every time.
        Rig r(sheetConfig("center"));
        r.sink.setOptions(hook());
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().size(), 2);
        r.sheet.forceHide();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(2), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
        r.sheet.forceHide();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(4), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
        QVERIFY(!r.sheet.isVisible());

        // Shown: one close and one hidden update, not two of each.
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().size(), 8);
        r.sheet.forceHide();
        QTRY_VERIFY(!r.sink.isBusy());
        QTest::qWait(100);
        QCOMPARE(calls().mid(8), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));

        // While a show is still on its way (slow eww): it ends hidden and closed.
        flag("slow", true);
        r.sheet.show();
        QVERIFY(r.sink.isBusy());
        r.sheet.forceHide();
        QTRY_VERIFY_WITH_TIMEOUT(!r.sink.isBusy(), 5000);
        const QStringList tail = calls().mid(10);
        QCOMPARE(tail.mid(tail.size() - 2), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
        QCOMPARE(tail.count(QStringLiteral("close pad-cheatsheet")), 1);
    }

    void forceHideAfterASkippedOpen()
    {
        // The daemon's view of the window can be wrong (an update failed, so
        // no open was sent, but a window is up anyway): the click still closes it.
        flag("fail-update", true);
        Rig r(sheetConfig("center"));
        r.sink.setOptions(hook());
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QVERIFY(!calls().contains(QStringLiteral("open pad-cheatsheet @center")));
        flag("fail-update", false);
        const int before = calls().size();
        r.sheet.forceHide();
        QTRY_VERIFY(!r.sink.isBusy());
        const QStringList tail = calls().mid(before);
        QCOMPARE(tail.size(), 2);
        QVERIFY(tail.contains(QStringLiteral("close pad-cheatsheet")));
        QVERIFY(tail.contains(QStringLiteral("update pad_sheet visible=false")));
    }

    void forceHideVariableOnly()
    {
        Rig r(sheetConfig("center"));
        EwwHook h;
        h.enabled = true;
        r.sink.setOptions(h);
        QTRY_VERIFY(!r.sink.isBusy());
        r.sheet.forceHide();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls(), (QStringList{QStringLiteral("update pad_sheet visible=false"), QStringLiteral("update pad_sheet visible=false")}));
        r.sink.setOptions(EwwHook{});  // off: a click changes nothing in eww
        r.sheet.forceHide();
        QTest::qWait(100);
        QCOMPARE(calls().size(), 2);
    }

    void finishHidesWithoutEventLoop()
    {
        Rig r(sheetConfig("center"));
        r.sink.setOptions(hook());
        r.sheet.show();
        QTRY_VERIFY(!r.sink.isBusy());
        QCOMPARE(calls().size(), 4);
        flag("slow", true);
        r.engine.setActiveWindow(kOther);
        QTRY_VERIFY(r.sink.isBusy());  // a slow update in flight
        r.sink.finish();               // waits for it, then hides; no event loop runs
        QVERIFY(!r.sink.isBusy());
        QCOMPARE(calls().mid(4), (QStringList{QStringLiteral("update pad_sheet visible=true"), QStringLiteral("close pad-cheatsheet"),
                                              QStringLiteral("update pad_sheet visible=false")}));
        // A hung eww does not hold up the exit for long.
        Rig r2(sheetConfig("center"));
        r2.sink.setOptions(hook(QString()));
        r2.sheet.show();
        QElapsedTimer t;
        t.start();
        r2.sink.finish(400);
        QVERIFY(t.elapsed() < 1500);
        QVERIFY(!r2.sink.isBusy());
    }
};

QTEST_GUILESS_MAIN(TestEww)
#include "tst_eww.moc"
