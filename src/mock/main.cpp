// SPDX-License-Identifier: GPL-2.0-or-later
// mock-kdenlive: serves org.kde.kdenlive.ControlSurface1 so the daemon can be
// exercised without a patched Kdenlive. Console commands on stdin:
//   focus <timeline|clipMonitor|projectMonitor|effectStack|automationEditor|bin>
//   effect <id>      e.g. effect lift_gamma_gain   ("effect -" clears)
//   param <name>     e.g. param level              ("param -" clears)
//   tool <select|razor|ripple|roll|slip|slide>
//   context <json>   replace the whole context
//   state            print the editing state
#include "kdenlivecontract.h"
#include "mockkdenlive.h"

#include <QCommandLineParser>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSocketNotifier>
#include <cstdio>
#include <unistd.h>

using namespace cs;

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    QCommandLineParser p;
    p.setApplicationDescription(QStringLiteral("Mock Kdenlive control-surface endpoint (org.kde.kdenlive.ControlSurface1)"));
    p.addHelpOption();
    QCommandLineOption serviceOpt(QStringLiteral("service"), QStringLiteral("bus name (default org.kde.kdenlive-<own pid>)"), QStringLiteral("name"));
    QCommandLineOption delayOpt(QStringLiteral("apply-delay"), QStringLiteral("simulated GUI cost per apply batch in ms"), QStringLiteral("ms"), QStringLiteral("0"));
    p.addOptions({serviceOpt, delayOpt});
    p.process(app);

    MockKdenlive mock;
    mock.setPrint(true);
    mock.setApplyDelayMs(p.value(delayOpt).toInt());
    QDBusConnection bus = QDBusConnection::sessionBus();
    const QString service = p.value(serviceOpt).isEmpty() ? contract::kServicePrefix + QString::number(QCoreApplication::applicationPid()) : p.value(serviceOpt);
    if (!mock.registerOn(bus) || !bus.registerService(service)) {
        std::fprintf(stderr, "cannot register %s %s: %s\n", qPrintable(service), qPrintable(contract::kPath), qPrintable(bus.lastError().message()));
        return 1;
    }
    std::printf("mock-kdenlive serving %s %s %s\n", qPrintable(service), qPrintable(contract::kPath), qPrintable(contract::kInterface));
    std::fflush(stdout);

    QSocketNotifier in(0, QSocketNotifier::Read);
    QObject::connect(&in, &QSocketNotifier::activated, &app, [&] {
        char buf[4096];
        const ssize_t n = ::read(0, buf, sizeof buf - 1);
        if (n <= 0) {
            in.setEnabled(false);
            return;
        }
        for (const QString &line : QString::fromUtf8(buf, int(n)).split(QLatin1Char('\n'), Qt::SkipEmptyParts)) {
            const QString cmd = line.section(QLatin1Char(' '), 0, 0);
            const QString arg = line.section(QLatin1Char(' '), 1).trimmed();
            if (cmd == QLatin1String("focus") || cmd == QLatin1String("tool")) {
                mock.setContextValue(cmd, arg);
            } else if (cmd == QLatin1String("effect")) {
                mock.setContextValue(QStringLiteral("effect"), arg == QLatin1String("-") ? QVariant() : QVariantMap{{QStringLiteral("id"), arg}});
            } else if (cmd == QLatin1String("param")) {
                mock.setContextValue(QStringLiteral("param"), arg == QLatin1String("-") ? QVariant() : QVariantMap{{QStringLiteral("name"), arg}});
            } else if (cmd == QLatin1String("context")) {
                mock.setContext(QJsonDocument::fromJson(arg.toUtf8()).object().toVariantMap());
            } else if (cmd == QLatin1String("state")) {
                std::printf("%s\n", QJsonDocument(QJsonObject::fromVariantMap(mock.state())).toJson(QJsonDocument::Compact).constData());
            } else {
                std::printf("unknown command: %s\n", qPrintable(cmd));
            }
            std::printf("context %s\n", QJsonDocument(QJsonObject::fromVariantMap(mock.context())).toJson(QJsonDocument::Compact).constData());
            std::fflush(stdout);
        }
    });
    return app.exec();
}
