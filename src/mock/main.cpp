// SPDX-License-Identifier: GPL-2.0-or-later
// mock-kdenlive: serves org.kde.kdenlive.ControlSurface1 (contract revision 2)
// so the daemon can be exercised without a patched Kdenlive.
//   --stage 1|2|3   advertise MR1 (transport/zoom/actions), MR2 (+parameters,
//                   wheels) or MR3 (+track, scroll, gain, trim). Default 3.
//   --off           own the service but not the object: Kdenlive's default.
//   --tick-ms <ms>  simulate playback: advance the playhead every <ms>
//                   (context emission stays capped at 30 Hz).
// Console commands on stdin:
//   focus <timeline|clipMonitor|projectMonitor|effectStack|bin>
//   wheel <lift|gamma|gain>|off   focus a Lift/Gamma/Gain wheel: publishes
//                       effect, colorWheel and the three colorWheels handles
//   hover <lift|gamma|gain>|off   hovered colour wheel (hoveredColorWheel)
//   param <level|opacity>|-       focused scalar parameter
//   keyframes <level|opacity> on|off   multi-key parameter (keys at 0 and 100)
//   play on|off         playback state (live grading vs multi-key edits)
//   grouped on|off      selection is a group: parameter edits are refused
//   track <trk-4|trk-3|trk-7|trk-8>   focused track (MR3)
//   clip <clip-21|clip-22|clip-31>|-  selected clip: clipGain and trim handles (MR3)
//   history <label>     an unrelated undo entry (ends gestures, new epoch)
//   tool <select|razor|ripple|roll|slip|slide>
//   dialog on|off       modal dialog
//   position <frame>    playhead (serial only; ends editing gestures)
//   state               print the editing state
#include "kdenlivecontract.h"
#include "mockkdenlive.h"

#include <QCommandLineParser>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSocketNotifier>
#include <QTimer>
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
    QCommandLineOption tickOpt(QStringLiteral("tick-ms"), QStringLiteral("simulate playback, one frame every <ms> (0 = stopped)"), QStringLiteral("ms"), QStringLiteral("0"));
    p.addOptions({serviceOpt, delayOpt, stageOpt, offOpt, tickOpt});
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

    QTimer playback;
    playback.setTimerType(Qt::PreciseTimer);
    QObject::connect(&playback, &QTimer::timeout, &mock, [&mock] { mock.setPosition(mock.context().value(QStringLiteral("position")).toInt() + 1); });
    if (p.value(tickOpt).toInt() > 0) {
        mock.setPlaying(true);
        playback.start(p.value(tickOpt).toInt());
    }

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
                mock.focusWheels(arg == QLatin1String("off") || arg == QLatin1String("-") ? QString() : arg == QLatin1String("on") ? QStringLiteral("lift") : arg);
            } else if (cmd == QLatin1String("hover")) {
                mock.hoverWheel(arg == QLatin1String("off") || arg == QLatin1String("-") ? QString() : arg == QLatin1String("on") ? QStringLiteral("gain") : arg);
            } else if (cmd == QLatin1String("param")) {
                mock.focusParam(arg == QLatin1String("-") || arg == QLatin1String("off") ? QString() : arg);
            } else if (cmd == QLatin1String("keyframes")) {
                mock.setParamMultiKey(arg.section(QLatin1Char(' '), 0, 0), arg.section(QLatin1Char(' '), 1) == QLatin1String("on"));
            } else if (cmd == QLatin1String("play")) {
                mock.setPlaying(on);
            } else if (cmd == QLatin1String("track")) {
                mock.focusTrack(arg);
            } else if (cmd == QLatin1String("clip")) {
                mock.selectClip(arg == QLatin1String("-") || arg == QLatin1String("off") ? QString() : arg);
            } else if (cmd == QLatin1String("source")) {
                mock.setSourceOpen(on);
            } else if (cmd == QLatin1String("trimming")) {
                mock.setTrimmingPreview(on);
            } else if (cmd == QLatin1String("drag")) {
                mock.setDragging(on);
            } else if (cmd == QLatin1String("grouped")) {
                mock.setGroupedPropagation(on);
            } else if (cmd == QLatin1String("history")) {
                mock.addUnrelatedHistory(arg.isEmpty() ? QStringLiteral("user edit") : arg);
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
