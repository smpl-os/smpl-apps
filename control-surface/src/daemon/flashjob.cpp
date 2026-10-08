// SPDX-License-Identifier: GPL-2.0-or-later
#include "flashjob.h"

#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QPointer>
#include <QProcess>
#include <QStandardPaths>
#include <QTemporaryFile>
#include <QTimer>

namespace cs {

QStringList FlashSettings::defaultImageDirs()
{
    return {QStandardPaths::writableLocation(QStandardPaths::GenericDataLocation) + QStringLiteral("/control-surface/firmware"),
            QStringLiteral("/usr/share/control-surface/firmware")};
}

FlashJob::FlashJob(const QString &id, const FlashSettings &settings, const QString &image, const QString &sha256, bool dryRun, QObject *parent)
    : QObject(parent)
    , m_id(id)
    , m_image(image)
    , m_sha(sha256.toLower())
    , m_s(settings)
    , m_dry(dryRun)
    , m_probe([] { return listUsbDevices(); })
    , m_timer(new QTimer(this))
{
    m_timer->setInterval(qMax(10, m_s.pollMs));
    connect(m_timer, &QTimer::timeout, this, &FlashJob::poll);
    m_runner = processRunner(m_s.tool);
}

FlashJob::~FlashJob() = default;

FlashJob::Runner FlashJob::processRunner(const QString &tool, int timeoutMs)
{
    return [tool, timeoutMs](const QString &image, Done done) {
        auto *p = new QProcess;
        p->setProcessChannelMode(QProcess::MergedChannels);
        auto *watchdog = new QTimer(p);
        watchdog->setSingleShot(true);
        QObject::connect(watchdog, &QTimer::timeout, p, [p] { p->kill(); });
        QObject::connect(p, &QProcess::finished, p, [p, done](int code, QProcess::ExitStatus st) {
            done(st == QProcess::NormalExit ? code : -1, QString::fromUtf8(p->readAll()));
            p->deleteLater();
        });
        QObject::connect(p, &QProcess::errorOccurred, p, [p, done](QProcess::ProcessError e) {
            if (e == QProcess::FailedToStart) {
                done(-1, QStringLiteral("cannot start the flash tool: %1").arg(p->errorString()));
                p->deleteLater();
            }
        });
        watchdog->start(timeoutMs);
        p->start(tool, {QStringLiteral("flash"), image});
    };
}

QJsonObject FlashJob::toJson() const
{
    QJsonObject o{{QStringLiteral("id"), m_id},
                  {QStringLiteral("image"), m_image},
                  {QStringLiteral("dryRun"), m_dry},
                  {QStringLiteral("phase"), m_phase},
                  {QStringLiteral("message"), m_message},
                  {QStringLiteral("finished"), m_finished},
                  {QStringLiteral("ok"), m_ok}};
    for (auto it = m_result.begin(); it != m_result.end(); ++it) {
        o.insert(it.key(), it.value());
    }
    return o;
}

void FlashJob::step(const QString &phase, const QString &message)
{
    m_phase = phase;
    m_message = message;
    Q_EMIT progress(m_id, phase, message);
}

void FlashJob::finish(bool ok, const QString &phase, const QString &message)
{
    if (m_finished) {
        return;
    }
    m_timer->stop();
    m_finished = true;
    m_ok = ok;
    if (m_released) {
        m_released = false;
        Q_EMIT reacquireDevice();
    }
    step(phase, message);
    Q_EMIT finished(m_id, ok);
}

void FlashJob::fail(const QString &message)
{
    finish(false, QStringLiteral("failed"), message);
}

QStringList FlashJob::checkImage(QString *warning)
{
    QStringList problems;
    const QFileInfo fi(m_image);
    if (!fi.isAbsolute()) {
        problems << QStringLiteral("image path must be absolute");
        return problems;
    }
    if (!fi.exists() || !fi.isFile()) {
        problems << QStringLiteral("image not found: %1").arg(m_image);
        return problems;
    }
    const QString real = fi.canonicalFilePath();
    bool inDir = false;
    for (const QString &d : m_s.imageDirs) {
        const QString dir = QFileInfo(d).canonicalFilePath();
        if (!dir.isEmpty() && real.startsWith(dir + QLatin1Char('/'))) {
            inDir = true;
        }
    }
    if (!inDir) {
        problems << QStringLiteral("image is not in an allowed firmware directory (%1)").arg(m_s.imageDirs.join(QStringLiteral(", ")));
    }
    // Read the bytes once, from the resolved file, and judge only those.
    QFile f(real);
    if (!f.open(QIODevice::ReadOnly)) {
        problems << QStringLiteral("cannot read the image: %1").arg(f.errorString());
        return problems;
    }
    const QByteArray data = f.read(m_s.maxImageBytes + 1);
    if (data.isEmpty() || data.size() > m_s.maxImageBytes) {
        problems << QStringLiteral("image size %1 is outside 1..%2 bytes").arg(qMax(qint64(data.size()), fi.size())).arg(m_s.maxImageBytes);
        return problems;
    }
    const QString got = QString::fromLatin1(QCryptographicHash::hash(data, QCryptographicHash::Sha256).toHex());
    if (got != m_sha) {
        problems << QStringLiteral("sha256 mismatch: image is %1").arg(got);
    }
    const QFileInfo tool(m_s.tool);
    if (m_s.tool.isEmpty() || !tool.isFile() || !tool.isExecutable()) {
        const QString t = QStringLiteral("flash tool not found: %1").arg(m_s.tool.isEmpty() ? QStringLiteral("(none configured)") : m_s.tool);
        if (m_dry) {
            *warning = t;
        } else {
            problems << t;
        }
    }
    if (!problems.isEmpty() || m_dry) {
        return problems;
    }
    QString dir = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
    if (dir.isEmpty()) {
        dir = QDir::tempPath();
    }
    // QTemporaryFile creates the file 0600; the .bin suffix tells wchisp it is raw.
    m_copy = std::make_unique<QTemporaryFile>(dir + QStringLiteral("/control-surface-flash-XXXXXX.bin"));
    if (!m_copy->open() || m_copy->write(data) != data.size() || !m_copy->flush()) {
        problems << QStringLiteral("cannot stage the image in %1").arg(dir);
        m_copy.reset();
    }
    return problems;
}

QString FlashJob::portOf(const UsbDeviceInfo &d)
{
    return d.sysPath.section(QLatin1Char('/'), -1);  // "1-4"; empty when unknown
}

QString FlashJob::bootloaderKey(const UsbDeviceInfo &d) const
{
    return QStringLiteral("%1-%2").arg(d.busnum).arg(d.devnum);
}

void FlashJob::start()
{
    step(QStringLiteral("checking"), QStringLiteral("checking %1").arg(m_image));
    QString warning;
    const QStringList problems = checkImage(&warning);
    if (!problems.isEmpty()) {
        fail(problems.join(QStringLiteral("; ")));
        return;
    }
    if (m_dry) {
        QStringList notes;
        if (!m_s.allowed) {
            notes << QStringLiteral("real flashing is disabled (start the daemon with --allow-flash)");
        }
        if (!warning.isEmpty()) {
            notes << warning;
        }
        m_result.insert(QStringLiteral("notes"), QJsonArray::fromStringList(notes));
        step(QStringLiteral("release-grab"), QStringLiteral("dry run: would release the pad"));
        step(QStringLiteral("waiting-bootloader"), m_requestBootloader ? QStringLiteral("dry run: would ask the firmware to switch to the bootloader")
                                                                      : QStringLiteral("dry run: would wait for the user to hold the top-left key while plugging the pad in"));
        step(QStringLiteral("flashing"), QStringLiteral("dry run: would run %1 flash %2").arg(m_s.tool, m_image));
        step(QStringLiteral("waiting-device"), QStringLiteral("dry run: would wait for 1189:8890"));
        step(QStringLiteral("verifying"), QStringLiteral("dry run: would compare the firmware reported by the pad"));
        finish(true, QStringLiteral("done"), notes.isEmpty() ? QStringLiteral("dry run passed") : QStringLiteral("dry run passed with notes: %1").arg(notes.join(QStringLiteral("; "))));
        return;
    }
    if (!m_s.allowed) {
        fail(QStringLiteral("real flashing is disabled (start the daemon with --allow-flash)"));
        return;
    }
    for (const UsbDeviceInfo &d : m_probe()) {
        if (isWchIsp(d)) {
            m_oldBootloaders.insert(bootloaderKey(d));  // possibly wedged: wait for a fresh one
        } else if (d.vendor == QLatin1String("1189") && d.product == QLatin1String("8890")) {
            m_oldPads.insert(bootloaderKey(d));
        }
    }
    m_released = true;
    Q_EMIT releaseDevice();
    step(QStringLiteral("release-grab"), QStringLiteral("released the pad"));
    m_deadline = QDateTime::currentMSecsSinceEpoch() + m_s.bootloaderTimeoutMs;
    if (!m_requestBootloader) {
        step(QStringLiteral("waiting-bootloader"), QStringLiteral("unplug the pad, hold the top-left key, plug it in, then let go"));
        m_timer->start();
        return;
    }
    step(QStringLiteral("waiting-bootloader"), QStringLiteral("asking the firmware to switch to the bootloader"));
    // Give the daemon a moment to let go of the pad before the request.
    QTimer::singleShot(300, this, [this] {
        if (m_finished) {
            return;
        }
        QString why;
        if (!m_requestBootloader(&why)) {
            step(QStringLiteral("waiting-bootloader"),
                 QStringLiteral("the firmware did not switch (%1): unplug the pad, hold the top-left key, plug it in, then let go").arg(why));
        }
        m_timer->start();
    });
}

void FlashJob::poll()
{
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    if (m_phase == QLatin1String("waiting-bootloader")) {
        // wchisp has no stable way to name a device (-d is a libusb index), so
        // it opens the first WCH ISP device it finds. Only flash when the new
        // session is the one and only WCH ISP device on the system.
        QList<UsbDeviceInfo> isp;
        const UsbDeviceInfo *fresh = nullptr;
        const QList<UsbDeviceInfo> devices = m_probe();
        for (const UsbDeviceInfo &d : devices) {
            if (isWchIsp(d)) {
                isp << d;
            }
        }
        for (const UsbDeviceInfo &d : std::as_const(isp)) {
            if (!m_oldBootloaders.contains(bootloaderKey(d))) {
                fresh = &d;
            }
        }
        if (fresh && isp.size() > 1) {
            const QString note = QStringLiteral("%1 WCH bootloader devices are connected; unplug all but the pad (the flash tool cannot tell them apart)").arg(isp.size());
            if (note != m_waitNote) {
                m_waitNote = note;
                step(QStringLiteral("waiting-bootloader"), note);
            }
        } else if (fresh) {
            m_timer->stop();
            m_bootloaderPort = portOf(*fresh);
            step(QStringLiteral("flashing"), QStringLiteral("bootloader session %1; writing %2").arg(bootloaderKey(*fresh), m_image));
            QPointer<FlashJob> self(this);
            m_runner(m_copy ? m_copy->fileName() : m_image, [self, this](int code, const QString &output) {
                if (!self || m_finished) {
                    return;
                }
                m_result.insert(QStringLiteral("toolExitCode"), code);
                m_result.insert(QStringLiteral("toolOutput"), output.right(4000));
                if (code != 0) {
                    fail(QStringLiteral("the flash tool failed (exit %1); the pad stays in the bootloader: replug it and try again").arg(code));
                    return;
                }
                step(QStringLiteral("waiting-device"), QStringLiteral("flashed; waiting for the pad (let go of all keys)"));
                m_deadline = QDateTime::currentMSecsSinceEpoch() + m_s.deviceTimeoutMs;
                m_timer->start();
            });
            return;
        }
        if (now >= m_deadline) {
            fail(QStringLiteral("no bootloader session within %1 s").arg(m_s.bootloaderTimeoutMs / 1000));
        }
        return;
    }
    if (m_phase == QLatin1String("waiting-device")) {
        for (const UsbDeviceInfo &d : m_probe()) {
            // The flashed pad comes back on the bootloader's port; another pad
            // that was already there does not count.
            const bool samePort = !m_bootloaderPort.isEmpty() && portOf(d) == m_bootloaderPort;
            const bool isNew = !m_oldPads.contains(bootloaderKey(d)) && (m_bootloaderPort.isEmpty() || portOf(d).isEmpty());
            if (d.vendor == QLatin1String("1189") && d.product == QLatin1String("8890") && (samePort || isNew)) {
                m_timer->stop();
                const FirmwareInfo fw = classifyFirmware(d);
                m_result.insert(QStringLiteral("firmware"), fw.toJson());
                step(QStringLiteral("verifying"), QStringLiteral("pad is back: %1 %2").arg(fw.type, fw.version));
                finish(true, QStringLiteral("done"), QStringLiteral("flashed; the pad runs %1 %2").arg(fw.type, fw.version));
                return;
            }
        }
        if (now >= m_deadline) {
            fail(QStringLiteral("the pad did not come back within %1 s; replug it (if it shows as 4348:55e0, a key was held)").arg(m_s.deviceTimeoutMs / 1000));
        }
    }
}

bool FlashJob::cancel()
{
    if (m_finished) {
        return false;
    }
    if (m_phase == QLatin1String("flashing") || m_phase == QLatin1String("verifying")) {
        return false;  // never interrupt a write
    }
    finish(false, QStringLiteral("cancelled"), QStringLiteral("cancelled"));
    return true;
}

} // namespace cs
