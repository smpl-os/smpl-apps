// SPDX-License-Identifier: GPL-2.0-or-later
// mock-control-surfaced: org.smplos.ControlSurface1 with a simulated pad, for
// developing the smplOS Settings "Keypad" app and the bar icon without
// hardware. Never touches a device and never runs a flash tool.
#include "mocksurface.h"

#include <QCommandLineParser>
#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDBusConnection>
#include <QDir>
#include <QFile>
#include <QSocketNotifier>
#include <QStandardPaths>
#include <csignal>
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
// SIGINT/SIGTERM end the event loop, so the mock's destructor hides the
// cheatsheet in eww.
void installSignalHandlers(QCoreApplication &app)
{
    if (::socketpair(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0, g_sigFd) != 0) {
        return;
    }
    auto *sn = new QSocketNotifier(g_sigFd[0], QSocketNotifier::Read, &app);
    QObject::connect(sn, &QSocketNotifier::activated, &app, [] {
        char c;
        [[maybe_unused]] auto r = ::read(g_sigFd[0], &c, 1);
        QCoreApplication::exit(0);
    });
    struct sigaction sa {};
    sa.sa_handler = onSignal;
    sigemptyset(&sa.sa_mask);
    ::sigaction(SIGINT, &sa, nullptr);
    ::sigaction(SIGTERM, &sa, nullptr);
}
} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    QCoreApplication::setApplicationName(QStringLiteral("mock-control-surfaced"));
    QCoreApplication::setApplicationVersion(QStringLiteral("0.1.0"));

    QString base = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
    if (base.isEmpty()) {
        base = QDir::tempPath();
    }
    base += QStringLiteral("/control-surface-mock");

    QCommandLineParser p;
    p.setApplicationDescription(QStringLiteral(
        "Simulated control-surfaced: serves org.smplos.ControlSurface1 plus org.smplos.ControlSurface1.Mock\n"
        "(/org/smplos/ControlSurface/Mock: Plug, Unplug, Press, Hold, Release, Turn, Focus,\n"
        "SetPluginStatus, EnterBootloader, SetFlashOutcome, TakeKeys; signal Dispatched)."));
    p.addHelpOption();
    p.addVersionOption();
    QCommandLineOption configOpt(QStringLiteral("config"), QStringLiteral("config file (default: a copy of the example in %1)").arg(base), QStringLiteral("path"),
                                 base + QStringLiteral("/config.jsonc"));
    QCommandLineOption fwDirOpt(QStringLiteral("firmware-dir"), QStringLiteral("simulated firmware images"), QStringLiteral("dir"), base + QStringLiteral("/firmware"));
    QCommandLineOption boardOpt(QStringLiteral("board"), QStringLiteral("board profile the pad reports"), QStringLiteral("id"), QStringLiteral("sy181-15k3e"));
    QCommandLineOption fwOpt(QStringLiteral("firmware"), QStringLiteral("control-surface | openmacropad | stock"), QStringLiteral("type"), QStringLiteral("control-surface"));
    QCommandLineOption unpluggedOpt(QStringLiteral("unplugged"), QStringLiteral("start with no pad"));
    QCommandLineOption ewwOpt(QStringLiteral("eww"), QStringLiteral("push the cheatsheet to eww, as control-surfaced run --eww"));
    QCommandLineOption ewwWindowOpt(QStringLiteral("eww-window"), QStringLiteral("open/close this eww window with the cheatsheet (implies --eww)"), QStringLiteral("name"));
    QCommandLineOption ewwConfigOpt(QStringLiteral("eww-config"), QStringLiteral("eww's config directory (implies --eww)"), QStringLiteral("dir"));
    p.addOptions({configOpt, fwDirOpt, boardOpt, fwOpt, unpluggedOpt, ewwOpt, ewwWindowOpt, ewwConfigOpt});
    p.process(app);
    installSignalHandlers(app);

    MockSurface::Options o;
    o.configPath = p.value(configOpt);
    o.firmwareDir = p.value(fwDirOpt);
    o.board = p.value(boardOpt);
    o.firmware = p.value(fwOpt);
    o.plugged = !p.isSet(unpluggedOpt);
    o.eww.enabled = p.isSet(ewwOpt) || p.isSet(ewwWindowOpt) || p.isSet(ewwConfigOpt);
    o.eww.window = p.value(ewwWindowOpt);
    o.eww.configDir = expandHome(p.value(ewwConfigOpt));

    QDir().mkpath(QFileInfo(o.configPath).absolutePath());
    if (!QFile::exists(o.configPath)) {
        QFile::copy(QStringLiteral(":/control-surface/config.example.jsonc"), o.configPath);
        QFile(o.configPath).setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner);
    }
    QDir().mkpath(o.firmwareDir);
    const QString image = o.firmwareDir + QStringLiteral("/control-surface-mock.bin");
    if (!QFile::exists(image)) {
        QFile f(image);
        if (f.open(QIODevice::WriteOnly)) {
            QByteArray data(2048, '\0');
            for (int i = 0; i < data.size(); ++i) {
                data[i] = char(i * 7 + 3);
            }
            f.write(data);
        }
    }
    QFile img(image);
    const QString sha = img.open(QIODevice::ReadOnly) ? QString::fromLatin1(QCryptographicHash::hash(img.readAll(), QCryptographicHash::Sha256).toHex())
                                                     : QStringLiteral("(unreadable)");

    MockSurface mock(o);
    QString err;
    if (!mock.registerOn(QDBusConnection::sessionBus(), true, &err)) {
        std::fprintf(stderr, "%s\n", qPrintable(err));
        return 1;
    }
    std::printf("mock-control-surfaced: %s on the session bus\n  config   %s\n  image    %s\n  sha256   %s\n",
                SettingsService::kService, qPrintable(o.configPath), qPrintable(image), qPrintable(sha));
    std::fflush(stdout);
    return app.exec();
}
