// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "usbinfo.h"

#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QStringList>
#include <functional>

class QTimer;

namespace cs {

struct FlashSettings {
    bool allowed = false;             // real flashing; dry runs are always allowed
    QString tool;                     // wchisp
    QStringList imageDirs;            // images must live in one of these
    qint64 maxImageBytes = 14336;     // CH552 user code flash
    int bootloaderTimeoutMs = 120000; // for the user to replug with the boot key held
    int deviceTimeoutMs = 30000;      // for the new firmware to enumerate
    int pollMs = 100;
    static QStringList defaultImageDirs();
};

// One firmware flash, driven by the settings API. It never forces the
// bootloader: the user enters it (boot key held at plug-in), the job notices a
// new 4348:55e0 session and flashes it once. The tool is always called as
// `<tool> flash <image>`, which writes code flash only (never the config
// registers). Phases: checking, release-grab, waiting-bootloader, flashing,
// waiting-device, verifying, then done | failed | cancelled.
class FlashJob : public QObject
{
    Q_OBJECT
public:
    using Probe = std::function<QList<UsbDeviceInfo>()>;
    using Done = std::function<void(int exitCode, const QString &output)>;
    using Runner = std::function<void(const QString &image, Done done)>;

    FlashJob(const QString &id, const FlashSettings &settings, const QString &image, const QString &sha256, bool dryRun, QObject *parent = nullptr);
    void setProbe(Probe p) { m_probe = std::move(p); }
    void setRunner(Runner r) { m_runner = std::move(r); }
    // Open firmware can switch to the ROM bootloader on request, so the user
    // need not hold the boot key. Returns false (with a reason) otherwise.
    using BootloaderRequest = std::function<bool(QString *error)>;
    void setBootloaderRequest(BootloaderRequest r) { m_requestBootloader = std::move(r); }
    static Runner processRunner(const QString &tool, int timeoutMs = 120000);

    void start();
    bool cancel();  // false while the tool is writing

    QString id() const { return m_id; }
    QString phase() const { return m_phase; }
    bool isFinished() const { return m_finished; }
    bool succeeded() const { return m_ok; }
    bool dryRun() const { return m_dry; }
    QJsonObject toJson() const;

Q_SIGNALS:
    void progress(const QString &id, const QString &phase, const QString &message);
    void releaseDevice();
    void reacquireDevice();
    void finished(const QString &id, bool ok);

private:
    void step(const QString &phase, const QString &message);
    void fail(const QString &message);
    void finish(bool ok, const QString &phase, const QString &message);
    QStringList checkImage(QString *warning) const;
    void poll();
    QString bootloaderKey(const UsbDeviceInfo &d) const;

    QString m_id, m_image, m_sha;
    FlashSettings m_s;
    bool m_dry = false;
    bool m_released = false;
    bool m_finished = false;
    bool m_ok = false;
    QString m_phase, m_message;
    QJsonObject m_result;
    QSet<QString> m_oldBootloaders;
    Probe m_probe;
    Runner m_runner;
    BootloaderRequest m_requestBootloader;
    QTimer *m_timer;
    qint64 m_deadline = 0;
};

} // namespace cs
