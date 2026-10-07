// SPDX-License-Identifier: GPL-2.0-or-later
// ch552-padprog: write the control-surface code scheme into a CH552 macro pad.
// Only binding, empty-key, ping and commit frames are ever produced; every frame
// is re-checked against an allow-list right before it is written.
#include "ch552proto.h"
#include "hidrawdev.h"
#include "scheme.h"

#include <QCommandLineParser>
#include <QCoreApplication>
#include <QDateTime>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

#include <cerrno>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <poll.h>
#include <unistd.h>

using namespace ch552;

namespace {

struct Step {
    QString what;
    Frame frame;
    int settleMs;
};

QString hexBytes(const unsigned char *p, int n)
{
    QString s;
    for (int i = 0; i < n; ++i) {
        s += QString::asprintf(i ? " %02x" : "%02x", p[i]);
    }
    return s;
}

std::vector<Step> buildPlan(Generation g, bool blank, std::pair<int, int> slotRange = {1, 24}, int commitSettleMs = 120)
{
    std::vector<Step> plan;
    plan.push_back({QStringLiteral("ping"), pingFrame(), 50});
    for (const auto &sc : defaultScheme()) {
        if (sc.slot < slotRange.first || sc.slot > slotRange.second) {
            continue;
        }
        const auto frames = blank ? emptyKey(g, sc.slot) : keyBinding(g, sc.slot, {sc.chord});
        for (std::size_t i = 0; i < frames.size(); ++i) {
            const bool commit = frames[i][0] == 0xAA;
            const QString what = blank ? QStringLiteral("slot %1 empty").arg(sc.slot)
                                       : QStringLiteral("slot %1 -> %2").arg(sc.slot).arg(QString::fromStdString(sc.name));
            plan.push_back({commit ? what + QStringLiteral(" commit") : what, frames[i], commit ? commitSettleMs : 30});
        }
    }
    return plan;
}

void printPlan(const std::vector<Step> &plan, Generation g)
{
    std::printf("report id %d, %zu frames (first 8 of 64 data bytes shown)\n", reportIdFor(g), plan.size());
    for (const auto &s : plan) {
        std::printf("  %-28s %s\n", qPrintable(s.what), hex(s.frame).c_str());
    }
}

int listDevices(const QString &serial)
{
    const auto nodes = findPadHidraw("1189", "8890", serial.toStdString());
    if (nodes.empty()) {
        std::printf("no 1189:8890 hidraw nodes found\n");
        return 1;
    }
    for (const auto &n : nodes) {
        const auto info = parseDescriptor(n.descriptor);
        const auto gen = generationFor(info);
        std::printf("%s if=%d serial=%s usb=%s out=%dbits in=%dbits reportIds=%s config=%s\n", n.devnode.c_str(), n.interfaceNumber,
                    n.serial.c_str(), n.usbPath.c_str(), info.outputBits, info.inputBits, info.hasReportIds ? "yes" : "no",
                    gen ? (*gen == Generation::Rid0 ? "rid0" : "rid3") : "-");
    }
    return 0;
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    QCoreApplication::setApplicationName(QStringLiteral("ch552-padprog"));
    QCommandLineParser p;
    p.setApplicationDescription(QStringLiteral(
        "Program the 24-slot control-surface scheme (F14-F19 x {none,shift,ctrl,alt}) into a 1189:8890 CH552 pad.\n"
        "Commands: list | plan | flash | blank"));
    p.addHelpOption();
    p.addPositionalArgument(QStringLiteral("command"), QStringLiteral("list, plan, flash or blank"));
    QCommandLineOption serialOpt(QStringLiteral("serial"), QStringLiteral("USB serial to require"), QStringLiteral("serial"), QStringLiteral("key153"));
    QCommandLineOption devOpt(QStringLiteral("device"), QStringLiteral("hidraw node (must still match VID:PID/interface 1)"), QStringLiteral("path"));
    QCommandLineOption logOpt(QStringLiteral("log"), QStringLiteral("write a JSON record of every frame and device reply"), QStringLiteral("file"));
    QCommandLineOption yesOpt(QStringLiteral("yes"), QStringLiteral("actually write to the device"));
    QCommandLineOption genOpt(QStringLiteral("generation"), QStringLiteral("plan only: rid0 or rid3"), QStringLiteral("gen"), QStringLiteral("rid0"));
    QCommandLineOption slotsOpt(QStringLiteral("slots"), QStringLiteral("only these slots: N or A-B within 1-24 (default 1-24)"), QStringLiteral("range"),
                                QStringLiteral("1-24"));
    QCommandLineOption settleOpt(QStringLiteral("settle-ms"), QStringLiteral("pause after each commit frame, 0-10000 ms (default 120)"), QStringLiteral("ms"),
                                 QStringLiteral("120"));
    p.addOptions({serialOpt, devOpt, logOpt, yesOpt, genOpt, slotsOpt, settleOpt});
    p.process(app);
    const QStringList args = p.positionalArguments();
    const QString cmd = args.value(0, QStringLiteral("plan"));
    const auto slotRange = parseSlotRange(p.value(slotsOpt).toStdString());
    if (!slotRange) {
        std::fprintf(stderr, "--slots: expected N or A-B within 1-24, got '%s'\n", qPrintable(p.value(slotsOpt)));
        return 2;
    }
    bool settleOk = false;
    const int settleMs = p.value(settleOpt).toInt(&settleOk);
    if (!settleOk || settleMs < 0 || settleMs > 10000) {
        std::fprintf(stderr, "--settle-ms: expected 0-10000, got '%s'\n", qPrintable(p.value(settleOpt)));
        return 2;
    }

    if (cmd == QLatin1String("list")) {
        return listDevices(p.value(serialOpt));
    }
    if (cmd == QLatin1String("plan")) {
        const Generation g = p.value(genOpt) == QLatin1String("rid3") ? Generation::Rid3 : Generation::Rid0;
        printPlan(buildPlan(g, false, *slotRange, settleMs), g);
        std::printf("\nexpected positions if slots are keys 1-15 then knobs (ccw,press,cw):\n");
        for (const auto &sc : defaultScheme()) {
            const auto a = slotTarget(Numbering::KeysThenKnobs, sc.slot);
            const auto b = slotTarget(Numbering::VendorTwelve, sc.slot);
            std::printf("  slot %2d %-10s A:%s.%s  B:%s.%s\n", sc.slot, sc.name.c_str(), a.control.c_str(), a.role.c_str(), b.control.c_str(),
                        b.role.c_str());
        }
        return 0;
    }
    if (cmd != QLatin1String("flash") && cmd != QLatin1String("blank")) {
        p.showHelp(2);
    }

    const auto nodes = findPadHidraw("1189", "8890", p.value(serialOpt).toStdString());
    const HidrawNode *target = nullptr;
    for (const auto &n : nodes) {
        if (n.interfaceNumber != 1) {
            continue;
        }
        if (p.isSet(devOpt) && QString::fromStdString(n.devnode) != p.value(devOpt)) {
            continue;
        }
        target = &n;
    }
    if (!target) {
        std::fprintf(stderr, "refusing: no interface-1 hidraw of 1189:8890 serial %s%s\n", qPrintable(p.value(serialOpt)),
                     p.isSet(devOpt) ? " at the given --device" : "");
        return 3;
    }
    const auto info = parseDescriptor(target->descriptor);
    const auto gen = generationFor(info);
    if (!gen) {
        std::fprintf(stderr, "refusing: unexpected configuration report descriptor (out=%d bits)\n", info.outputBits);
        return 4;
    }
    const auto plan = buildPlan(*gen, cmd == QLatin1String("blank"), *slotRange, settleMs);
    for (const auto &s : plan) {
        if (!isAllowedFrame(s.frame)) {
            std::fprintf(stderr, "internal error: frame not allowed: %s\n", hex(s.frame).c_str());
            return 5;
        }
    }
    std::printf("target %s (usb %s serial %s, interface 1, %s)\n", target->devnode.c_str(), target->usbPath.c_str(), target->serial.c_str(),
                *gen == Generation::Rid0 ? "no report id" : "report id 3");
    printPlan(plan, *gen);
    if (!p.isSet(yesOpt)) {
        std::printf("\ndry run; pass --yes to write\n");
        return 0;
    }

    const int fd = ::open(target->devnode.c_str(), O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) {
        std::fprintf(stderr, "open %s: %s\n", target->devnode.c_str(), std::strerror(errno));
        return 6;
    }
    QJsonArray records;
    QElapsedTimer clock;
    clock.start();
    int failures = 0;
    int replies = 0;
    unsigned char rx[64];
    auto drain = [&](int waitMs, QJsonArray &into) {
        QElapsedTimer t;
        t.start();
        while (true) {
            const int left = waitMs - int(t.elapsed());
            if (left <= 0) {
                break;
            }
            pollfd pfd{fd, POLLIN, 0};
            if (::poll(&pfd, 1, left) <= 0) {
                break;
            }
            const ssize_t r = ::read(fd, rx, sizeof rx);
            if (r <= 0) {
                break;
            }
            ++replies;
            into.append(QJsonObject{{QStringLiteral("t_ms"), double(clock.elapsed())}, {QStringLiteral("hex"), hexBytes(rx, int(r))}});
        }
    };
    QJsonArray preReplies;
    drain(100, preReplies);  // anything already pending is not an ack of ours
    for (const auto &s : plan) {
        unsigned char buf[1 + kFrameSize];
        buf[0] = reportIdFor(*gen);
        std::memcpy(buf + 1, s.frame.data(), kFrameSize);
        const qint64 t0 = clock.elapsed();
        const ssize_t w = ::write(fd, buf, sizeof buf);
        const int err = w < 0 ? errno : 0;
        if (w != ssize_t(sizeof buf)) {
            ++failures;
        }
        QJsonArray acks;
        drain(s.settleMs, acks);
        records.append(QJsonObject{{QStringLiteral("what"), s.what},
                                   {QStringLiteral("t_ms"), double(t0)},
                                   {QStringLiteral("frame"), QString::fromStdString(hex(s.frame, 8))},
                                   {QStringLiteral("written"), double(w)},
                                   {QStringLiteral("error"), err ? QString::fromLocal8Bit(std::strerror(err)) : QString()},
                                   {QStringLiteral("replies"), acks}});
        std::printf("%-28s %s -> %zd%s%s\n", qPrintable(s.what), hex(s.frame).c_str(), w, err ? " " : "",
                    err ? std::strerror(err) : "");
        for (const auto &a : acks) {
            std::printf("    reply %s\n", qPrintable(a.toObject().value(QStringLiteral("hex")).toString()));
        }
        if (failures > 0 && w < 0 && (err == ENODEV || err == EIO)) {
            break;
        }
    }
    ::close(fd);
    std::printf("\n%d frames, %d write failures, %d device replies\n", int(plan.size()), failures, replies);

    if (p.isSet(logOpt)) {
        QJsonObject root{{QStringLiteral("tool"), QStringLiteral("ch552-padprog")},
                         {QStringLiteral("command"), cmd},
                         {QStringLiteral("when"), QDateTime::currentDateTime().toString(Qt::ISODateWithMs)},
                         {QStringLiteral("device"), QString::fromStdString(target->devnode)},
                         {QStringLiteral("usb"), QString::fromStdString(target->usbPath)},
                         {QStringLiteral("serial"), QString::fromStdString(target->serial)},
                         {QStringLiteral("reportId"), int(reportIdFor(*gen))},
                         {QStringLiteral("preexistingReplies"), preReplies},
                         {QStringLiteral("frames"), records},
                         {QStringLiteral("writeFailures"), failures},
                         {QStringLiteral("replies"), replies}};
        QFile f(p.value(logOpt));
        if (f.open(QIODevice::WriteOnly | QIODevice::Truncate)) {
            f.write(QJsonDocument(root).toJson());
        } else {
            std::fprintf(stderr, "cannot write log %s\n", qPrintable(p.value(logOpt)));
        }
    }
    return failures == 0 ? 0 : 7;
}
