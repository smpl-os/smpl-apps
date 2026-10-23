// SPDX-License-Identifier: GPL-2.0-or-later
#include "service.h"
#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusError>
#include <QDir>
#include <QJsonDocument>
#include <QTextStream>
#include <csignal>

namespace {
volatile std::sig_atomic_t stopping = 0;
void stop(int) { stopping = 1; }
} // namespace

int main(int argc, char **argv) {
    QCoreApplication app(argc, argv);
    QCoreApplication::setApplicationName(QStringLiteral("mouse-remapperd"));
    const auto args = app.arguments().mid(1);
    if (args == QStringList{"features", "--json"}) {
        QTextStream(stdout) << QJsonDocument(QJsonObject{{"apiVersion", 1},
                                                         {"vendor", "1532"},
                                                         {"product", "00b4"},
                                                         {"learning", true},
                                                         {"identify", true},
                                                         {"identifyWheels", true},
                                                         {"guidedCalibration", true},
                                                         {"guidedSetup", true},
                                                         {"passthrough", true},
                                                         {"kdenliveApi", true},
                                                         {"appProfiles", "hyprland"}})
                                   .toJson(QJsonDocument::Compact)
                            << '\n';
        return 0;
    }
    if (args != QStringList{"run"}) {
        QTextStream(args.contains("--help") ? stdout : stderr) << "Usage: mouse-remapperd run | features --json\n"
                                                                  "Exact device: Razer Naga V2 HyperSpeed (1532:00b4).\n"
                                                                  "run grabs only this mouse's button interfaces; features opens no "
                                                                  "input.\n";
        return args.contains("--help") ? 0 : 2;
    }
    auto bus = QDBusConnection::sessionBus();
    const QString path = QDir::homePath() + "/.config/smplos/mouse.json";
    cs::mouse::Service service(path);
    if (!bus.registerService(QStringLiteral("org.smplos.MouseRemapper1")) ||
        !bus.registerObject(QStringLiteral("/org/smplos/MouseRemapper1"), &service,
                            QDBusConnection::ExportAllSlots | QDBusConnection::ExportAllSignals)) {
        QTextStream(stderr) << "Mouse D-Bus registration failed: " << bus.lastError().message() << '\n';
        return 1;
    }
    std::signal(SIGTERM, stop);
    std::signal(SIGINT, stop);
    QTimer signalTimer;
    QObject::connect(&signalTimer, &QTimer::timeout, &app, [&] {
        if (stopping)
            app.quit();
    });
    signalTimer.start(100);
    QObject::connect(&app, &QCoreApplication::aboutToQuit, &service, &cs::mouse::Service::stop);
    service.start();
    return app.exec();
}
