// SPDX-License-Identifier: GPL-2.0-or-later
#include "windowtracker.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalServer>
#include <QLocalSocket>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>
#include <QTimer>

using namespace cs;

// Fake Hyprland: .socket2.sock pushes events, .socket.sock answers j/activewindow.
class FakeHyprland : public QObject
{
    Q_OBJECT
public:
    explicit FakeHyprland(const QString &dir)
    {
        events.listen(dir + QStringLiteral("/.socket2.sock"));
        requests.listen(dir + QStringLiteral("/.socket.sock"));
        connect(&events, &QLocalServer::newConnection, this, [this] {
            while (auto *s = events.nextPendingConnection()) {
                clients << s;
            }
        });
        connect(&requests, &QLocalServer::newConnection, this, [this] {
            while (auto *s = requests.nextPendingConnection()) {
                connect(s, &QLocalSocket::readyRead, s, [this, s] {
                    const QByteArray req = s->readAll();
                    ++queries;
                    lastRequest = req;
                    // Answer with the window focused when the request arrived, optionally late.
                    const QByteArray answer = QJsonDocument(active).toJson(QJsonDocument::Compact);
                    QTimer::singleShot(replyDelayMs, s, [s, answer] {
                        s->write(answer);
                        s->flush();
                        s->disconnectFromServer();
                    });
                });
            }
        });
    }
    void push(const QByteArray &line)
    {
        for (auto *c : std::as_const(clients)) {
            c->write(line + '\n');
            c->flush();
        }
    }
    void dropClients()
    {
        for (auto *c : std::as_const(clients)) {
            c->disconnectFromServer();
        }
        clients.clear();
    }
    QLocalServer events, requests;
    QList<QLocalSocket *> clients;
    QJsonObject active;
    int queries = 0;
    int replyDelayMs = 0;
    QByteArray lastRequest;
};

class TestHyprland : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void tracksActiveWindowWithPid()
    {
        QTemporaryDir dir;
        FakeHyprland hypr(dir.path());
        hypr.active = QJsonObject{{QStringLiteral("class"), QStringLiteral("firefox")}, {QStringLiteral("title"), QStringLiteral("start")},
                                  {QStringLiteral("pid"), 11}, {QStringLiteral("address"), QStringLiteral("0xaaa")}};
        HyprlandTracker t(dir.path());
        QSignalSpy spy(&t, &WindowTracker::activeWindowChanged);
        t.start();
        QTRY_COMPARE(t.current().pid, qint64(11));  // initial query on connect
        QCOMPARE(hypr.lastRequest, QByteArray("j/activewindow"));
        QTRY_COMPARE(hypr.clients.size(), 1);

        hypr.active = QJsonObject{{QStringLiteral("class"), QStringLiteral("org.kde.kdenlive")}, {QStringLiteral("title"), QStringLiteral("a, b - Kdenlive")},
                                  {QStringLiteral("pid"), 4242}, {QStringLiteral("address"), QStringLiteral("0xbbb")}};
        hypr.push("activewindow>>org.kde.kdenlive,a, b - Kdenlive");
        hypr.push("activewindowv2>>bbb");
        QTRY_COMPARE(t.current().pid, qint64(4242));
        QCOMPARE(t.current().cls, QStringLiteral("org.kde.kdenlive"));
        QCOMPARE(t.current().title, QStringLiteral("a, b - Kdenlive"));

        hypr.push("windowtitlev2>>bbb,renamed, with comma");
        QTRY_COMPARE(t.current().title, QStringLiteral("renamed, with comma"));
        hypr.push("windowtitlev2>>ccc,other window");
        QTest::qWait(50);
        QCOMPARE(t.current().title, QStringLiteral("renamed, with comma"));

        hypr.active = QJsonObject{};  // nothing focused
        hypr.push("closewindow>>bbb");
        QTRY_COMPARE(t.current().cls, QString());
        QCOMPARE(t.current().pid, qint64(0));
    }
    void staleQueryReplyIsDropped()
    {
        QTemporaryDir dir;
        FakeHyprland hypr(dir.path());
        hypr.active = QJsonObject{{QStringLiteral("class"), QStringLiteral("a")}, {QStringLiteral("pid"), 1}, {QStringLiteral("address"), QStringLiteral("0x1")}};
        HyprlandTracker t(dir.path());
        t.start();
        QTRY_COMPARE(t.current().cls, QStringLiteral("a"));
        QTRY_COMPARE(hypr.clients.size(), 1);
        QStringList seen;
        connect(&t, &WindowTracker::activeWindowChanged, this, [&seen](const WindowInfo &w) { seen << w.cls; });
        hypr.replyDelayMs = 150;
        hypr.active = QJsonObject{{QStringLiteral("class"), QStringLiteral("b")}, {QStringLiteral("pid"), 2}, {QStringLiteral("address"), QStringLiteral("0x2")}};
        hypr.push("activewindow>>b,B");
        QTRY_COMPARE(hypr.queries, 2);  // query for b in flight (answers late with b)
        hypr.active = QJsonObject{{QStringLiteral("class"), QStringLiteral("c")}, {QStringLiteral("pid"), 3}, {QStringLiteral("address"), QStringLiteral("0x3")}};
        hypr.push("activewindow>>c,C");
        QTRY_COMPARE_WITH_TIMEOUT(t.current().pid, qint64(3), 2000);
        // After c was reported, the late answer for b must not flip focus back.
        QCOMPARE(seen.mid(seen.indexOf(QStringLiteral("c"))).count(QStringLiteral("b")), 0);
    }

    void provisionalEventCarriesNoStalePid()
    {
        HyprlandTracker t(QStringLiteral("/nonexistent"));
        t.handleEventLine(QStringLiteral("activewindow>>kitty,shell"));
        QCOMPARE(t.current().cls, QStringLiteral("kitty"));
        QCOMPARE(t.current().pid, qint64(0));
        t.handleEventLine(QStringLiteral("garbage line"));
        t.handleEventLine(QStringLiteral("workspace>>2"));
        QCOMPARE(t.current().cls, QStringLiteral("kitty"));
    }
    void reconnectsAfterRestart()
    {
        QTemporaryDir dir;
        auto *hypr = new FakeHyprland(dir.path());
        hypr->active = QJsonObject{{QStringLiteral("class"), QStringLiteral("a")}, {QStringLiteral("pid"), 1}};
        HyprlandTracker t(dir.path());
        t.start();
        QTRY_COMPARE(hypr->clients.size(), 1);
        hypr->dropClients();
        delete hypr;
        hypr = new FakeHyprland(dir.path());
        hypr->active = QJsonObject{{QStringLiteral("class"), QStringLiteral("b")}, {QStringLiteral("pid"), 2}};
        QTRY_COMPARE_WITH_TIMEOUT(t.current().cls, QStringLiteral("b"), 5000);
        delete hypr;
    }
};

QTEST_GUILESS_MAIN(TestHyprland)
#include "tst_hyprland.moc"
