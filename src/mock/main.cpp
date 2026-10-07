// SPDX-License-Identifier: GPL-2.0-or-later
// mock-kdenlive: serves org.kde.kdenlive.ControlSurface1 (contract revision 2)
// so the daemon can be exercised without a patched Kdenlive.
//   --stage 1|2|3   advertise MR1 (transport/zoom/actions), MR2 (+parameters,
//                   wheels) or MR3 (+track, scroll, gain, trim). Default 3.
//   --off           own the service but not the object: Kdenlive's default.
// Console commands on stdin:
//   focus <timeline|clipMonitor|projectMonitor|effectStack|bin>
//   wheel on|off        focused Lift/Gamma/Gain effect (effect, colorWheel)
//   hover on|off        hovered colour wheel (hoveredColorWheel)
//   param <name>|-      focused scalar parameter
//   tool <select|razor|ripple|roll|slip|slide>
//   dialog on|off       modal dialog
//   position <frame>    playhead (serial only)
//   state               print the editing state
#include "kdenlivecontract.h"
#include "mockkdenlive.h"

#include <QCommandLineParser>
#include <QCoreApplication>
#include <QDBusConnection>
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
    p.setApplicationDescription(QStringLiteral("Mock Kdenlive control-surface endpoint (org.kde.kdenlive.ControlSurface1, revision 2)"));
    p.addHelpOption();
    QCommandLineOption serviceOpt(QStringLiteral("service"), QStringLiteral("bus name (default org.kde.kdenlive-<own pid>)"), QStringLiteral("name"));
    QCommandLineOption delayOpt(QStringLiteral("apply-delay"), QStringLiteral("simulated GUI cost per apply batch in ms"), QStringLiteral("ms"), QStringLiteral("0"));
    QCommandLineOption stageOpt(QStringLiteral("stage"), QStringLiteral("advertised stage 1-3"), QStringLiteral("n"), QStringLiteral("3"));
    QCommandLineOption offOpt(QStringLiteral("off"), QStringLiteral("interface disabled (object absent), like Kdenlive's default"));
    p.addOptions({serviceOpt, delayOpt, stageOpt, offOpt});
    p.process(app);

    MockKdenlive mock;
    mock.setPrint(true);
    mock.setApplyDelayMs(p.value(delayOpt).toInt());
    mock.setStage(p.value(stageOpt).toInt());
    QDBusConnection bus = QDBusConnection::sessionBus();
    const QString service = p.value(serviceOpt).isEmpty() ? contract::kServicePrefix + QString::number(QCoreApplication::applicationPid()) : p.value(serviceOpt);
    if ((!p.isSet(offOpt) && !mock.registerOn(bus)) || !bus.registerService(service)) {
        std::fprintf(stderr, "cannot register %s %s: %s\n", qPrintable(service), qPrintable(contract::kPath), qPrintable(bus.lastError().message()));
        return 1;
    }
    std::printf("mock-kdenlive serving %s %s %s (stage %d)\n", qPrintable(service), p.isSet(offOpt) ? "(interface OFF)" : qPrintable(contract::kPath),
                qPrintable(contract::kInterface), mock.stage());
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
            const bool on = arg == QLatin1String("on");
            if (cmd == QLatin1String("focus") || cmd == QLatin1String("tool")) {
                mock.setContextValue(cmd, arg);
            } else if (cmd == QLatin1String("wheel")) {
                mock.setContextValue(QStringLiteral("effect"), on ? QVariant(QVariantMap{{QStringLiteral("target"), QStringLiteral("fx-1")}, {QStringLiteral("id"), QStringLiteral("lift_gamma_gain")}}) : QVariant());
                mock.setContextValue(QStringLiteral("colorWheel"), on ? QVariant(QVariantMap{{QStringLiteral("target"), QStringLiteral("cw-1")}, {QStringLiteral("axes"), QStringList{QStringLiteral("value"), QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")}}}) : QVariant());
            } else if (cmd == QLatin1String("hover")) {
                mock.setContextValue(QStringLiteral("hoveredColorWheel"), on ? QVariant(QVariantMap{{QStringLiteral("target"), QStringLiteral("cw-hover")}}) : QVariant());
            } else if (cmd == QLatin1String("param")) {
                mock.setContextValue(QStringLiteral("param"), arg == QLatin1String("-") ? QVariant() : QVariant(QVariantMap{{QStringLiteral("name"), arg}, {QStringLiteral("target"), QStringLiteral("par-") + arg}}));
            } else if (cmd == QLatin1String("dialog")) {
                mock.setContextValue(QStringLiteral("dialog"), on);
            } else if (cmd == QLatin1String("position")) {
                mock.setPosition(arg.toInt());
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
