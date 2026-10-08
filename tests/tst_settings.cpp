// SPDX-License-Identifier: GPL-2.0-or-later
// Settings API (org.smplos.ControlSurface1), its building blocks and the mock.
// Bus tests run only on a PRIVATE bus (ctest starts this under dbus-run-session).
#include "boardprofile.h"
#include "configedit.h"
#include "configstore.h"
#include "ewwmock.h"
#include "flashjob.h"
#include "inputmonitor.h"
#include "mocksurface.h"
#include "settingsservice.h"
#include "usbinfo.h"

#include <QCryptographicHash>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusArgument>
#include <QDBusMetaType>
#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

using namespace cs;

namespace {
const QByteArray kConfig = R"({"profiles":[{"name":"global","bindings":{"key1":"F13","key2":"F15","knob1.cw":"F16"}}]})";
const QByteArray kConfig2 = R"({"profiles":[{"name":"global","bindings":{"key1":"F20"}}]})";

void writeFile(const QString &path, const QByteArray &data)
{
    QDir().mkpath(QFileInfo(path).absolutePath());
    QFile f(path);
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write(data);
}

QByteArray readFile(const QString &path)
{
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
}

QJsonObject obj(const QString &json)
{
    return QJsonDocument::fromJson(json.toUtf8()).object();
}

QString sha(const QByteArray &d)
{
    return QString::fromLatin1(QCryptographicHash::hash(d, QCryptographicHash::Sha256).toHex());
}

void usbDir(const QString &root, const QString &name, const QMap<QString, QString> &attrs)
{
    const QString dir = root + QStringLiteral("/bus/usb/devices/") + name;
    for (auto it = attrs.begin(); it != attrs.end(); ++it) {
        writeFile(dir + QLatin1Char('/') + it.key(), it.value().toUtf8() + "\n");
    }
}

UsbDeviceInfo usb(const QString &vid, const QString &pid, const QString &mfr = {}, const QString &prod = {}, const QString &bcd = QStringLiteral("0100"), int ifaces = 1, int devnum = 9)
{
    UsbDeviceInfo d;
    d.vendor = vid;
    d.product = pid;
    d.manufacturer = mfr;
    d.productName = prod;
    d.bcdDevice = bcd;
    d.interfaces = ifaces;
    d.busnum = 1;
    d.devnum = devnum;
    d.serial = QStringLiteral("key153");
    return d;
}

QDBusConnection bus(const QString &name)
{
    return QDBusConnection::connectToBus(QDBusConnection::SessionBus, name);
}
} // namespace

// Records D-Bus signals of org.smplos.ControlSurface1 as seen by a client.
class Receiver : public QObject
{
    Q_OBJECT
public:
    Receiver(QDBusConnection c, const QString &path = QLatin1String(SettingsService::kPath)) : m_c(c)
    {
        const QString s = QLatin1String(SettingsService::kService), i = QLatin1String(SettingsService::kInterface);
        m_c.connect(s, path, i, QStringLiteral("InputEvent"), this, SLOT(onInput(QString, QString, int)));
        m_c.connect(s, path, i, QStringLiteral("IdentifyChanged"), this, SLOT(onIdentify(bool)));
        m_c.connect(s, path, i, QStringLiteral("DeviceChanged"), this, SLOT(onDevice(bool, QString)));
        m_c.connect(s, path, i, QStringLiteral("ConfigChanged"), this, SLOT(onConfig(QString)));
        m_c.connect(s, path, i, QStringLiteral("ConfigRejected"), this, SLOT(onRejected(QString)));
        m_c.connect(s, path, i, QStringLiteral("PluginsChanged"), this, SLOT(onPlugins()));
        m_c.connect(s, path, i, QStringLiteral("FlashProgress"), this, SLOT(onFlash(QString, QString, QString)));
        m_c.connect(s, path, i, QStringLiteral("CheatsheetChanged"), this, SLOT(onSheet(QString)));
        m_c.connect(s, path, i, QStringLiteral("CheatsheetVisibilityChanged"), this, SLOT(onSheetVisible(bool)));
        m_c.connect(s, path, QStringLiteral("org.freedesktop.DBus.Properties"), QStringLiteral("PropertiesChanged"), this, SLOT(onProps(QString, QVariantMap, QStringList)));
        m_c.connect(s, QLatin1String(MockSurface::kMockPath), QStringLiteral("org.smplos.ControlSurface1.Mock"), QStringLiteral("Dispatched"), this,
                    SLOT(onDispatched(QString, QString, QString)));
    }
    QDBusMessage call(const QString &method, const QVariantList &args = {}, const QString &path = QLatin1String(SettingsService::kPath),
                      const QString &iface = QLatin1String(SettingsService::kInterface))
    {
        auto m = QDBusMessage::createMethodCall(QLatin1String(SettingsService::kService), path, iface, method);
        m.setArguments(args);
        return m_c.call(m, QDBus::BlockWithGui, 5000);
    }
    QJsonObject json(const QString &method, const QVariantList &args = {})
    {
        return obj(call(method, args).arguments().value(0).toString());
    }
    QDBusMessage mock(const QString &method, const QVariantList &args = {})
    {
        return call(method, args, QLatin1String(MockSurface::kMockPath), QStringLiteral("org.smplos.ControlSurface1.Mock"));
    }
    QVariantMap props()
    {
        const QVariant v = call(QStringLiteral("GetAll"), {QLatin1String(SettingsService::kInterface)}, QLatin1String(SettingsService::kPath),
                                QStringLiteral("org.freedesktop.DBus.Properties"))
                               .arguments()
                               .value(0);
        return v.canConvert<QDBusArgument>() ? qdbus_cast<QVariantMap>(v.value<QDBusArgument>()) : v.toMap();
    }

    QStringList inputs, devices, configs, rejected, flash, dispatched, sheets;
    QList<bool> sheetVisible;
    QList<bool> identify;
    QVariantMap changedProps;
    int plugins = 0;

public Q_SLOTS:
    void onInput(const QString &slot, const QString &event, int delta) { inputs << QStringLiteral("%1 %2 %3").arg(slot, event).arg(delta); }
    void onIdentify(bool on) { identify << on; }
    void onDevice(bool present, const QString &fw) { devices << QStringLiteral("%1 %2").arg(present).arg(fw); }
    void onConfig(const QString &hash) { configs << hash; }
    void onRejected(const QString &e) { rejected << e; }
    void onPlugins() { ++plugins; }
    void onFlash(const QString &id, const QString &phase, const QString &) { flash << id + QLatin1Char(':') + phase; }
    void onProps(const QString &, const QVariantMap &changed, const QStringList &) { changedProps.insert(changed); }
    void onDispatched(const QString &slot, const QString &binding, const QString &) { dispatched << slot + QLatin1Char('=') + binding; }
    void onSheet(const QString &json) { sheets << json; }
    void onSheetVisible(bool on) { sheetVisible << on; }

private:
    QDBusConnection m_c;
};

class TestSettings : public QObject
{
    Q_OBJECT

private:
    bool privateBus() const { return qEnvironmentVariable("CS_PRIVATE_BUS") == QLatin1String("1"); }

    struct FlashFixture {
        QTemporaryDir dir;
        QString image, imageSha, tool;
        FlashSettings settings;
        QList<UsbDeviceInfo> devices;
        QString ranImage;
        FlashJob::Done pending;
        FlashFixture()
        {
            image = dir.path() + QStringLiteral("/fw/pad.bin");
            QByteArray data(1000, '\x5a');
            writeFile(image, data);
            imageSha = sha(data);
            tool = dir.path() + QStringLiteral("/fake-wchisp");
            writeFile(tool, "#!/bin/sh\necho \"fake $@\"\nexit 0\n");
            QFile(tool).setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
            settings.allowed = true;
            settings.tool = tool;
            settings.imageDirs = {dir.path() + QStringLiteral("/fw")};
            settings.pollMs = 10;
            settings.bootloaderTimeoutMs = 3000;
            settings.deviceTimeoutMs = 3000;
        }
        FlashJob *job(bool dry, const QString &img = {}, const QString &hash = {})
        {
            auto *j = new FlashJob(QStringLiteral("j"), settings, img.isEmpty() ? image : img, hash.isEmpty() ? imageSha : hash, dry);
            j->setProbe([this] { return devices; });
            j->setRunner([this](const QString &i, FlashJob::Done d) {
                ranImage = i;
                pending = d;
            });
            return j;
        }
    };

private Q_SLOTS:
    void cleanup()
    {
        // A failed check returns early: never leave a name or connection behind.
        for (const char *n : {"srv1", "srv2", "cli1", "msrv", "mcli", "monsrv", "moncli", "gone", "csrv", "ccli"}) {
            QDBusConnection c = QDBusConnection(QLatin1String(n));
            if (c.isConnected()) {
                c.unregisterService(QLatin1String(SettingsService::kService));
                QDBusConnection::disconnectFromBus(QLatin1String(n));
            }
        }
    }

    // ------------------------------------------------------------------ usbinfo
    void usbListingAndClassification()
    {
        QTemporaryDir t;
        usbDir(t.path(), QStringLiteral("1-4"), {{QStringLiteral("idVendor"), QStringLiteral("1189")}, {QStringLiteral("idProduct"), QStringLiteral("8890")},
                                                 {QStringLiteral("manufacturer"), QStringLiteral("OpenMacroPad")}, {QStringLiteral("product"), QStringLiteral("Control Surface 15+3")},
                                                 {QStringLiteral("serial"), QStringLiteral("key153")}, {QStringLiteral("bcdDevice"), QStringLiteral("0200")},
                                                 {QStringLiteral("busnum"), QStringLiteral("1")}, {QStringLiteral("devnum"), QStringLiteral("30")},
                                                 {QStringLiteral("bNumInterfaces"), QStringLiteral(" 1")}});
        usbDir(t.path(), QStringLiteral("1-4:1.0"), {{QStringLiteral("bInterfaceNumber"), QStringLiteral("00")}});
        usbDir(t.path(), QStringLiteral("1-5"), {{QStringLiteral("idVendor"), QStringLiteral("4348")}, {QStringLiteral("idProduct"), QStringLiteral("55E0")},
                                                 {QStringLiteral("busnum"), QStringLiteral("1")}, {QStringLiteral("devnum"), QStringLiteral("31")}});
        const auto all = listUsbDevices(t.path());
        QCOMPARE(all.size(), 2);
        QCOMPARE(all[0].productName, QStringLiteral("Control Surface 15+3"));
        QCOMPARE(all[0].interfaces, 1);
        QCOMPARE(all[1].product, QStringLiteral("55e0"));  // lower-cased

        const FirmwareInfo cs = classifyFirmware(all[0]);
        QCOMPARE(cs.type, QStringLiteral("control-surface"));
        QCOMPARE(cs.version, QStringLiteral("2.0"));
        QCOMPARE(cs.board, QStringLiteral("sy181-15k3e"));
        QCOMPARE(classifyFirmware(all[1]).type, QStringLiteral("bootloader"));
        QCOMPARE(classifyFirmware(usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("wch.cn"), QStringLiteral("CH552"), QStringLiteral("0100"), 4)).type,
                 QStringLiteral("stock"));
        QCOMPARE(classifyFirmware(usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("SY181"), QStringLiteral("Macropad 12+3"))).type,
                 QStringLiteral("openmacropad"));
        QCOMPARE(classifyFirmware(usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("Other"), QStringLiteral("X"))).type, QStringLiteral("unknown"));
        QCOMPARE(classifyFirmware(usb(QStringLiteral("046d"), QStringLiteral("c52b"))).type, QStringLiteral("unknown"));
        const FirmwareInfo v = classifyFirmware(usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("OpenMacroPad"), QStringLiteral("Control Surface 3+1"), QStringLiteral("0213")));
        QCOMPARE(v.version, QStringLiteral("2.13"));
        QVERIFY(v.board.isEmpty());
        QVERIFY(!usbDeviceAt(t.path() + QStringLiteral("/nope")));
    }

    // ------------------------------------------------------------- board profiles
    void boardProfiles()
    {
        const auto m = builtinBoardProfile(QStringLiteral("sy181-15k3e"));
        QVERIFY(m);
        QCOMPARE(m->keys.size(), 15);
        QCOMPARE(m->knobs.size(), 3);
        QCOMPARE(m->slotCount(), 24);
        QCOMPARE(m->rows, 3);
        QCOMPARE(m->columns, 6);
        QCOMPARE(m->keys[0].control, QStringLiteral("key1"));
        QCOMPARE(m->keys[6].row, 1);
        QCOMPARE(m->keys[6].column, 1);
        QCOMPARE(m->knobs[0].ccw, 15);
        QCOMPARE(m->knobs[2].cw, 23);
        QCOMPARE(m->knobs[1].column, 5);
        QCOMPARE(m->source, QStringLiteral("measured"));

        QSet<QString> ids;
        for (const BoardProfile &p : builtinBoardProfiles()) {
            QVERIFY(!ids.contains(p.id));
            ids.insert(p.id);
            QSet<int> used;
            for (const auto &k : p.keys) {
                used.insert(k.slot);
            }
            for (const auto &n : p.knobs) {
                used << n.ccw << n.press << n.cw;
            }
            QCOMPARE(used.size(), p.slotCount());  // every slot once
            QVERIFY(p.slotCount() <= 25);
            const QJsonObject j = p.toJson();
            QCOMPARE(j.value(QStringLiteral("keys")).toArray().size(), p.keys.size());
            QCOMPARE(j.value(QStringLiteral("slotCount")).toInt(), p.slotCount());
        }
        const auto knobsOnly = gridProfile(QStringLiteral("k"), QStringLiteral("k"), 0, 2, 3);
        QCOMPARE(knobsOnly.columns, 1);
        QCOMPARE(knobsOnly.knobs[1].ccw, 3);

        BoardProfile c = profileForControls({QStringLiteral("key1"), QStringLiteral("key15"), QStringLiteral("knob3")}, QStringLiteral("config"));
        QCOMPARE(c.id, QStringLiteral("sy181-15k3e"));
        QCOMPARE(c.source, QStringLiteral("config"));
        c = profileForControls({QStringLiteral("key3"), QStringLiteral("knob1")}, QStringLiteral("config"));
        QCOMPARE(c.keys.size(), 3);
        QCOMPARE(c.knobs.size(), 1);
        QCOMPARE(c.columns, 4);
    }

    // -------------------------------------------------------------- config store
    void configStore()
    {
        QTemporaryDir t;
        const QString path = t.path() + QStringLiteral("/cs/config.jsonc");
        ConfigStore store(path);
        QVERIFY(!store.read().exists);

        auto v = store.validate("{not json");
        QVERIFY(!v.ok);
        QVERIFY(!v.errors.isEmpty());
        v = store.validate(kConfig);
        QVERIFY(v.ok);
        QCOMPARE(v.toJson().value(QStringLiteral("profiles")).toArray().size(), 1);

        // Create: only with an empty expected hash.
        auto w = store.write(kConfig, QStringLiteral("00"));
        QVERIFY(!w.ok);
        QCOMPARE(w.error, QStringLiteral("hash-mismatch"));
        QVERIFY(!QFile::exists(path));
        w = store.write(kConfig, QString());
        QVERIFY(w.ok);
        QCOMPARE(w.hash, ConfigStore::hashOf(kConfig));
        QVERIFY(w.backup.isEmpty());
        QCOMPARE(readFile(path), kConfig);

        // Invalid text: refused, file untouched.
        w = store.write("{\"profiles\":[{\"name\":\"x\",\"bindings\":{\"key99\":\"a\"}}]}", ConfigStore::hashOf(kConfig));
        QVERIFY(!w.ok);
        QCOMPARE(w.error, QStringLiteral("invalid"));
        QCOMPARE(readFile(path), kConfig);

        // Stale hash (someone edited by hand): refused.
        writeFile(path, kConfig + "\n");
        w = store.write(kConfig2, ConfigStore::hashOf(kConfig));
        QCOMPARE(w.error, QStringLiteral("hash-mismatch"));
        QCOMPARE(readFile(path), kConfig + "\n");

        // Current hash: written, previous kept.
        w = store.write(kConfig2, ConfigStore::hashOf(kConfig + "\n"));
        QVERIFY(w.ok);
        QCOMPARE(readFile(path), kConfig2);
        QCOMPARE(readFile(path + QStringLiteral(".bak")), kConfig + "\n");
        QCOMPARE(w.toJson().value(QStringLiteral("backup")).toString(), path + QStringLiteral(".bak"));

        // Same content: no-op, backup unchanged.
        w = store.write(kConfig2, ConfigStore::hashOf(kConfig2));
        QVERIFY(w.ok);
        QCOMPARE(readFile(path + QStringLiteral(".bak")), kConfig + "\n");

        // Too large.
        w = store.write(QByteArray(ConfigStore::kMaxBytes + 1, ' '), ConfigStore::hashOf(kConfig2));
        QCOMPARE(w.error, QStringLiteral("too-large"));
        QCOMPARE(readFile(path), kConfig2);
    }

    // ---------------------------------------------------------------- flash job
    void flashDryRun()
    {
        FlashFixture f;
        f.settings.allowed = false;
        std::unique_ptr<FlashJob> j(f.job(true));
        QSignalSpy prog(j.get(), &FlashJob::progress);
        QSignalSpy rel(j.get(), &FlashJob::releaseDevice);
        j->start();
        QVERIFY(j->isFinished());
        QVERIFY(j->succeeded());
        QStringList phases;
        for (const auto &a : prog) {
            phases << a.at(1).toString();
        }
        QCOMPARE(phases, (QStringList{QStringLiteral("checking"), QStringLiteral("release-grab"), QStringLiteral("waiting-bootloader"), QStringLiteral("flashing"),
                                      QStringLiteral("waiting-device"), QStringLiteral("verifying"), QStringLiteral("done")}));
        QCOMPARE(rel.count(), 0);
        QVERIFY(f.ranImage.isEmpty());  // never runs the tool
        QVERIFY(j->toJson().value(QStringLiteral("notes")).toArray().first().toString().contains(QStringLiteral("--allow-flash")));
    }

    void flashRefusals()
    {
        FlashFixture f;
        auto failsWith = [&](FlashJob *raw, const QString &text) {
            std::unique_ptr<FlashJob> j(raw);
            QSignalSpy rel(j.get(), &FlashJob::releaseDevice);
            j->start();
            QVERIFY(j->isFinished());
            QVERIFY(!j->succeeded());
            QCOMPARE(j->phase(), QStringLiteral("failed"));
            QVERIFY2(j->toJson().value(QStringLiteral("message")).toString().contains(text), qPrintable(j->toJson().value(QStringLiteral("message")).toString()));
            QCOMPARE(rel.count(), 0);  // the pad is never released for a refused job
        };
        failsWith(f.job(false, QString(), QString(64, QLatin1Char('0'))), QStringLiteral("sha256 mismatch"));
        failsWith(f.job(true, QString(), QString(64, QLatin1Char('0'))), QStringLiteral("sha256 mismatch"));
        writeFile(f.dir.path() + QStringLiteral("/elsewhere.bin"), QByteArray(1000, '\x5a'));
        failsWith(f.job(false, f.dir.path() + QStringLiteral("/elsewhere.bin")), QStringLiteral("allowed firmware directory"));
        failsWith(f.job(false, QStringLiteral("fw/pad.bin")), QStringLiteral("absolute"));
        failsWith(f.job(false, f.dir.path() + QStringLiteral("/fw/missing.bin")), QStringLiteral("not found"));
        QByteArray big(20000, '\x01');
        writeFile(f.dir.path() + QStringLiteral("/fw/big.bin"), big);
        failsWith(f.job(false, f.dir.path() + QStringLiteral("/fw/big.bin"), sha(big)), QStringLiteral("outside 1..14336"));
        // A symlink out of the firmware directory does not count as inside it.
        QFile::link(f.dir.path() + QStringLiteral("/elsewhere.bin"), f.dir.path() + QStringLiteral("/fw/link.bin"));
        failsWith(f.job(false, f.dir.path() + QStringLiteral("/fw/link.bin")), QStringLiteral("allowed firmware directory"));
        writeFile(f.dir.path() + QStringLiteral("/fw/locked.bin"), QByteArray(1000, '\x5a'));
        QFile(f.dir.path() + QStringLiteral("/fw/locked.bin")).setPermissions(QFileDevice::WriteOwner);
        if (!QFile(f.dir.path() + QStringLiteral("/fw/locked.bin")).open(QIODevice::ReadOnly)) {  // not as root
            failsWith(f.job(false, f.dir.path() + QStringLiteral("/fw/locked.bin")), QStringLiteral("cannot read"));
        }
        f.settings.allowed = false;
        failsWith(f.job(false), QStringLiteral("--allow-flash"));
        f.settings.allowed = true;
        f.settings.tool = f.dir.path() + QStringLiteral("/no-tool");
        failsWith(f.job(false), QStringLiteral("flash tool not found"));
    }

    void flashFullRun()
    {
        FlashFixture f;
        f.devices = {usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("wch.cn"), QStringLiteral("CH552"), QStringLiteral("0100"), 4, 7),
                     usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 5)};  // an old, possibly wedged session
        std::unique_ptr<FlashJob> j(f.job(false));
        QSignalSpy rel(j.get(), &FlashJob::releaseDevice);
        QSignalSpy acq(j.get(), &FlashJob::reacquireDevice);
        QSignalSpy done(j.get(), &FlashJob::finished);
        QSignalSpy prog(j.get(), &FlashJob::progress);
        j->start();
        QCOMPARE(rel.count(), 1);
        QCOMPARE(j->phase(), QStringLiteral("waiting-bootloader"));
        QTest::qWait(60);
        QCOMPARE(j->phase(), QStringLiteral("waiting-bootloader"));  // the old session is ignored
        // A new session while the old one is still there: wchisp would open the
        // first WCH ISP device it finds, so nothing is flashed yet.
        f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 5),
                     usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 6)};
        QTRY_VERIFY(prog.last().at(2).toString().contains(QStringLiteral("unplug all but the pad")));
        QTest::qWait(60);
        QCOMPARE(j->phase(), QStringLiteral("waiting-bootloader"));
        QVERIFY(f.ranImage.isEmpty());
        // The same with another WCH chip's ISP id.
        f.devices = {usb(QStringLiteral("1a86"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0100"), 1, 3),
                     usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 6)};
        QTest::qWait(60);
        QVERIFY(f.ranImage.isEmpty());
        // Only the new session left: flash it.
        f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 6)};
        QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
        QVERIFY(f.ranImage != f.image);  // a private copy of the verified bytes
        QCOMPARE(readFile(f.ranImage), readFile(f.image));
        QCOMPARE(int(QFile(f.ranImage).permissions() & (QFileDevice::ReadGroup | QFileDevice::ReadOther | QFileDevice::WriteGroup | QFileDevice::WriteOther)), 0);
        QVERIFY(!j->cancel());  // never interrupt the write
        f.devices = {usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("OpenMacroPad"), QStringLiteral("Control Surface 15+3"), QStringLiteral("0200"), 1, 8)};
        f.pending(0, QStringLiteral("Verify OK"));
        QTRY_VERIFY(j->isFinished());
        QVERIFY(j->succeeded());
        QCOMPARE(acq.count(), 1);
        QCOMPARE(done.count(), 1);
        QCOMPARE(j->toJson().value(QStringLiteral("firmware")).toObject().value(QStringLiteral("type")).toString(), QStringLiteral("control-surface"));
        QCOMPARE(j->toJson().value(QStringLiteral("toolOutput")).toString(), QStringLiteral("Verify OK"));
    }

    void flashUsesTheVerifiedBytes()
    {
        // The image is swapped (here: a symlink re-pointed outside the image
        // directory) while the job waits for the bootloader.
        FlashFixture f;
        const QString outside = f.dir.path() + QStringLiteral("/outside.bin");
        writeFile(outside, QByteArray(1000, 'E'));
        const QString link = f.dir.path() + QStringLiteral("/fw/link.bin");
        QVERIFY(QFile::link(f.image, link));
        std::unique_ptr<FlashJob> j(f.job(false, link));
        j->start();
        QCOMPARE(j->phase(), QStringLiteral("waiting-bootloader"));
        QVERIFY(QFile::remove(link));
        QVERIFY(QFile::link(outside, link));
        writeFile(f.image, QByteArray(1000, 'X'));  // and the original rewritten too
        f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"))};
        QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
        QCOMPARE(readFile(f.ranImage), QByteArray(1000, '\x5a'));
        const QString staged = f.ranImage;
        f.pending(0, QString());
        j.reset();
        QVERIFY(!QFile::exists(staged));  // the private copy goes with the job
    }

    void flashWaitsForThePadThatWasFlashed()
    {
        FlashFixture f;
        f.settings.deviceTimeoutMs = 300;
        UsbDeviceInfo other = usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("wch.cn"), QStringLiteral("CH552"), QStringLiteral("0100"), 4, 11);
        other.sysPath = QStringLiteral("/sys/bus/usb/devices/1-7");  // a second pad, already there
        f.devices = {other};
        std::unique_ptr<FlashJob> j(f.job(false));
        j->start();
        UsbDeviceInfo boot = usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 12);
        boot.sysPath = QStringLiteral("/sys/bus/usb/devices/1-4");
        f.devices = {other, boot};
        QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
        f.devices = {other};  // the flashed pad does not come back
        f.pending(0, QString());
        QTRY_VERIFY(j->isFinished());
        QVERIFY(!j->succeeded());  // the other pad is not mistaken for it
        // And when it does come back on its port:
        FlashFixture g;
        g.devices = {other};
        std::unique_ptr<FlashJob> k(g.job(false));
        k->start();
        g.devices = {other, boot};
        QTRY_COMPARE(k->phase(), QStringLiteral("flashing"));
        UsbDeviceInfo back = usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("OpenMacroPad"), QStringLiteral("Control Surface 15+3"), QStringLiteral("0200"), 1, 13);
        back.sysPath = QStringLiteral("/sys/bus/usb/devices/1-4");
        g.devices = {other, back};
        g.pending(0, QString());
        QTRY_VERIFY(k->isFinished());
        QVERIFY(k->succeeded());
        QCOMPARE(k->toJson().value(QStringLiteral("firmware")).toObject().value(QStringLiteral("type")).toString(), QStringLiteral("control-surface"));
    }

    void flashFailures()
    {
        {  // the tool fails
            FlashFixture f;
            std::unique_ptr<FlashJob> j(f.job(false));
            QSignalSpy acq(j.get(), &FlashJob::reacquireDevice);
            j->start();
            f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"))};
            QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
            f.pending(1, QStringLiteral("timeout"));
            QVERIFY(j->isFinished());
            QVERIFY(!j->succeeded());
            QCOMPARE(acq.count(), 1);
            QVERIFY(j->toJson().value(QStringLiteral("message")).toString().contains(QStringLiteral("stays in the bootloader")));
        }
        {  // nobody enters the bootloader
            FlashFixture f;
            f.settings.bootloaderTimeoutMs = 100;
            std::unique_ptr<FlashJob> j(f.job(false));
            QSignalSpy acq(j.get(), &FlashJob::reacquireDevice);
            j->start();
            QTRY_VERIFY(j->isFinished());
            QCOMPARE(j->phase(), QStringLiteral("failed"));
            QCOMPARE(acq.count(), 1);
        }
        {  // the pad does not come back
            FlashFixture f;
            f.settings.deviceTimeoutMs = 100;
            std::unique_ptr<FlashJob> j(f.job(false));
            j->start();
            f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"))};
            QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
            f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"))};  // still in the bootloader
            f.pending(0, QString());
            QTRY_VERIFY(j->isFinished());
            QVERIFY(j->toJson().value(QStringLiteral("message")).toString().contains(QStringLiteral("did not come back")));
        }
        {  // cancelled while waiting
            FlashFixture f;
            std::unique_ptr<FlashJob> j(f.job(false));
            QSignalSpy acq(j.get(), &FlashJob::reacquireDevice);
            j->start();
            QVERIFY(j->cancel());
            QCOMPARE(j->phase(), QStringLiteral("cancelled"));
            QCOMPARE(acq.count(), 1);
            QVERIFY(!j->cancel());
            f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"))};
            QTest::qWait(50);
            QVERIFY(f.ranImage.isEmpty());  // nothing runs after a cancel
        }
    }

    void flashAsksOpenFirmwareForTheBootloader()
    {
        {  // the firmware switches: no boot key needed
            FlashFixture f;
            std::unique_ptr<FlashJob> j(f.job(false));
            int asked = 0;
            j->setBootloaderRequest([&](QString *) {
                ++asked;
                f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"))};
                return true;
            });
            QSignalSpy prog(j.get(), &FlashJob::progress);
            j->start();
            QCOMPARE(asked, 0);  // the pad is released first
            QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
            QCOMPARE(asked, 1);
            QVERIFY(prog.at(2).at(2).toString().contains(QStringLiteral("asking the firmware")));
            f.devices = {usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("OpenMacroPad"), QStringLiteral("Control Surface 15+3"), QStringLiteral("0200"))};
            f.pending(0, QString());
            QTRY_VERIFY(j->succeeded());
        }
        {  // it does not: fall back to the boot key
            FlashFixture f;
            std::unique_ptr<FlashJob> j(f.job(false));
            j->setBootloaderRequest([](QString *why) {
                *why = QStringLiteral("no reply");
                return false;
            });
            QSignalSpy prog(j.get(), &FlashJob::progress);
            j->start();
            QTRY_VERIFY(prog.last().at(2).toString().contains(QStringLiteral("hold the top-left key")));
            QCOMPARE(j->phase(), QStringLiteral("waiting-bootloader"));
            f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"))};
            QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
            j.reset();  // destroyed mid-job: the pending tool callback must not crash
            f.pending(0, QString());
        }
        {  // a dry run says which way it would go
            FlashFixture f;
            std::unique_ptr<FlashJob> j(f.job(true));
            j->setBootloaderRequest([](QString *) { return true; });
            QSignalSpy prog(j.get(), &FlashJob::progress);
            j->start();
            QVERIFY(prog.at(2).at(2).toString().contains(QStringLiteral("would ask the firmware")));
        }
    }

    void flashProcessRunner()
    {
        FlashFixture f;
        int code = -2;
        QString out;
        FlashJob::processRunner(f.tool)(f.image, [&](int c, const QString &o) {
            code = c;
            out = o;
        });
        QTRY_COMPARE(code, 0);
        QCOMPARE(out.trimmed(), QStringLiteral("fake flash %1").arg(f.image));  // fixed arguments only
        code = -2;
        FlashJob::processRunner(f.dir.path() + QStringLiteral("/no-tool"))(f.image, [&](int c, const QString &o) {
            code = c;
            out = o;
        });
        QTRY_COMPARE(code, -1);
    }

    // ------------------------------------------------------- service, in process
    void serviceDirect()
    {
        QTemporaryDir t;
        const QString path = t.path() + QStringLiteral("/config.jsonc");
        writeFile(path, kConfig);
        SettingsService s(path);
        QSignalSpy input(&s, &SettingsService::InputEvent);

        // Live input is always published; actions are suppressed only while identifying.
        QVERIFY(!s.filterPadEvent(PadEvent{QStringLiteral("key7"), PadEvent::KeyDown, 0, 0}));
        QVERIFY(!s.filterPadEvent(PadEvent{QStringLiteral("knob2"), PadEvent::Turn, -1, 0}));
        QVERIFY(!s.filterPadEvent(PadEvent{QStringLiteral("knob2"), PadEvent::PressUp, 0, 0}));
        QCOMPARE(input.count(), 3);
        QCOMPARE(input.at(0), (QVariantList{QStringLiteral("key7"), QStringLiteral("press"), 0}));
        QCOMPARE(input.at(1), (QVariantList{QStringLiteral("knob2"), QStringLiteral("ccw"), -1}));
        QCOMPARE(input.at(2), (QVariantList{QStringLiteral("knob2"), QStringLiteral("release"), 0}));
        s.SetIdentify(true);
        QVERIFY(s.identifyActive());
        QVERIFY(s.filterPadEvent(PadEvent{QStringLiteral("knob3"), PadEvent::Turn, 1, 0}));
        QCOMPARE(input.last(), (QVariantList{QStringLiteral("knob3"), QStringLiteral("cw"), 1}));
        s.SetIdentify(false);
        QVERIFY(!s.filterPadEvent(PadEvent{QStringLiteral("key1"), PadEvent::KeyUp, 0, 0}));

        // JSON queries.
        QCOMPARE(obj(s.GetLayout()).value(QStringLiteral("ok")).toBool(), false);
        s.setFallbackLayout(profileForControls({QStringLiteral("key3"), QStringLiteral("knob1")}, QStringLiteral("config")));
        QCOMPARE(obj(s.GetLayout()).value(QStringLiteral("layout")).toObject().value(QStringLiteral("source")).toString(), QStringLiteral("config"));
        DeviceState d;
        d.present = true;
        d.usb = usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("OpenMacroPad"), QStringLiteral("Control Surface 15+3"), QStringLiteral("0200"));
        d.firmware = classifyFirmware(d.usb);
        QCOMPARE(d.firmware.version, QStringLiteral("2.0"));
        QCOMPARE(d.firmware.versionSource, QStringLiteral("bcdDevice"));
        s.setDevice(d);
        // The config's layout (an override, e.g. a variant) wins over the
        // firmware's board; both are reported, with the mismatch.
        QJsonObject layout = obj(s.GetLayout()).value(QStringLiteral("layout")).toObject();
        QCOMPARE(layout.value(QStringLiteral("id")).toString(), QStringLiteral("grid-3k1e"));
        QCOMPARE(layout.value(QStringLiteral("source")).toString(), QStringLiteral("config"));
        QCOMPARE(layout.value(QStringLiteral("firmwareLayout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));
        QCOMPARE(layout.value(QStringLiteral("firmwareLayout")).toObject().value(QStringLiteral("source")).toString(), QStringLiteral("firmware"));
        QCOMPARE(layout.value(QStringLiteral("matchesFirmware")).toBool(), false);
        QJsonObject st = obj(s.GetStatus());
        QCOMPARE(st.value(QStringLiteral("layout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("grid-3k1e"));
        QJsonArray warnings = st.value(QStringLiteral("config")).toObject().value(QStringLiteral("warnings")).toArray();
        QCOMPARE(warnings.size(), 1);
        QVERIFY(warnings.first().toString().startsWith(QLatin1String("layout: the config's grid-3k1e")));
        // Without an override the firmware's board is used.
        s.setFallbackLayout(profileForControls({QStringLiteral("key3"), QStringLiteral("knob1")}, QStringLiteral("hardware-map")));
        layout = obj(s.GetLayout()).value(QStringLiteral("layout")).toObject();
        QCOMPARE(layout.value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));
        QCOMPARE(layout.value(QStringLiteral("source")).toString(), QStringLiteral("firmware"));
        QCOMPARE(layout.value(QStringLiteral("matchesFirmware")).toBool(), true);
        QVERIFY(obj(s.GetStatus()).value(QStringLiteral("config")).toObject().value(QStringLiteral("warnings")).toArray().isEmpty());
        QCOMPARE(obj(s.GetDevice()).value(QStringLiteral("firmware")).toObject().value(QStringLiteral("type")).toString(), QStringLiteral("control-surface"));
        // The full version once GET_INFO answered (bcdDevice carries only 2.0).
        d.firmware.version = QStringLiteral("2.0.1");
        d.firmware.versionSource = QStringLiteral("GET_INFO");
        d.firmware.slotCount = 24;
        s.setDevice(d);
        const QJsonObject fwJson = obj(s.GetDevice()).value(QStringLiteral("firmware")).toObject();
        QCOMPARE(fwJson.value(QStringLiteral("version")).toString(), QStringLiteral("2.0.1"));
        QCOMPARE(fwJson.value(QStringLiteral("versionSource")).toString(), QStringLiteral("GET_INFO"));
        QCOMPARE(fwJson.value(QStringLiteral("slots")).toInt(), 24);
        QCOMPARE(obj(s.GetFirmwareStatus()).value(QStringLiteral("device")).toObject().value(QStringLiteral("firmware")).toObject().value(QStringLiteral("version")).toString(),
                 QStringLiteral("2.0.1"));
        QCOMPARE(obj(s.GetLayout()).value(QStringLiteral("layout")).toObject().value(QStringLiteral("firmwareSlots")).toInt(), 24);
        // ValidateConfig answers for the pad as it is: the layout a config would give.
        QJsonObject v = obj(s.ValidateConfig(QStringLiteral(R"({"layout": "generic-12k2e", "profiles": []})")));
        QCOMPARE(v.value(QStringLiteral("ok")).toBool(), true);
        QCOMPARE(v.value(QStringLiteral("layout")).toObject().value(QStringLiteral("source")).toString(), QStringLiteral("config"));
        QCOMPARE(v.value(QStringLiteral("layout")).toObject().value(QStringLiteral("matchesFirmware")).toBool(), false);
        QVERIFY(v.value(QStringLiteral("warnings")).toArray().last().toString().startsWith(QLatin1String("layout: the config's generic-12k2e")));
        v = obj(s.ValidateConfig(QStringLiteral(R"({"layout": "sy181-15k3e", "profiles": []})")));  // same slots: fine
        QCOMPARE(v.value(QStringLiteral("layout")).toObject().value(QStringLiteral("matchesFirmware")).toBool(), true);
        QVERIFY(v.value(QStringLiteral("warnings")).toArray().isEmpty());
        v = obj(s.ValidateConfig(QStringLiteral(R"({"profiles": []})")));
        QCOMPARE(v.value(QStringLiteral("layout")).toObject().value(QStringLiteral("source")).toString(), QStringLiteral("firmware"));
        QVERIFY(obj(s.ListBoardProfiles()).value(QStringLiteral("profiles")).toArray().size() >= 5);
        st = obj(s.GetStatus());
        QCOMPARE(st.value(QStringLiteral("apiVersion")).toInt(), 1);
        QCOMPARE(st.value(QStringLiteral("device")).toObject().value(QStringLiteral("present")).toBool(), true);
        QCOMPARE(st.value(QStringLiteral("layout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));

        // Plugins: a change signals once, a repeat does not.
        QSignalSpy plugins(&s, &SettingsService::PluginsChanged);
        s.setPlugins({PluginInfo{QStringLiteral("keys"), QStringLiteral("Keys"), QStringLiteral("keys"), QStringLiteral("ready"), {}, {}, {}}});
        s.setPlugins({PluginInfo{QStringLiteral("keys"), QStringLiteral("Keys"), QStringLiteral("keys"), QStringLiteral("ready"), {}, {}, {}}});
        QCOMPARE(plugins.count(), 1);
        QCOMPARE(obj(s.ListPlugins()).value(QStringLiteral("plugins")).toArray().first().toObject().value(QStringLiteral("status")).toString(), QStringLiteral("ready"));

        // Flash arguments.
        QCOMPARE(obj(s.StartFlash(QStringLiteral("/x.bin"), QStringLiteral("abc"), true)).value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(),
                 QStringLiteral("invalid-arguments"));
        QCOMPARE(obj(s.StartFlash(QStringLiteral("/x.bin"), QString(64, QLatin1Char('a')), false)).value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(),
                 QStringLiteral("not-allowed"));
    }

    void serviceConfig()
    {
        QTemporaryDir t;
        const QString path = t.path() + QStringLiteral("/config.jsonc");
        writeFile(path, kConfig);
        SettingsService s(path);
        QList<Config> applied;
        s.setConfigApplier([&](const Config &c) {
            applied << c;
            return QString();
        });
        s.setConfigState(ConfigStore::hashOf(kConfig), QString(), {});
        QSignalSpy changed(&s, &SettingsService::ConfigChanged);
        QSignalSpy rejected(&s, &SettingsService::ConfigRejected);

        QString p, h;
        QCOMPARE(s.GetConfig(p, h).toUtf8(), kConfig);
        QCOMPARE(p, path);
        QCOMPARE(h, ConfigStore::hashOf(kConfig));

        QVERIFY(!obj(s.ValidateConfig(QStringLiteral("{\"profiles\":[{\"name\":\"x\",\"bindings\":{\"key99\":\"a\"}}]}"))).value(QStringLiteral("ok")).toBool());
        QVERIFY(obj(s.ValidateConfig(QString::fromUtf8(kConfig2))).value(QStringLiteral("ok")).toBool());
        QVERIFY(applied.isEmpty());  // validation never applies

        QJsonObject r = obj(s.SetConfig(QString::fromUtf8(kConfig2), QStringLiteral("stale")));
        QCOMPARE(r.value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(), QStringLiteral("hash-mismatch"));
        QVERIFY(applied.isEmpty());
        r = obj(s.SetConfig(QString::fromUtf8(kConfig2), h));
        QVERIFY(r.value(QStringLiteral("ok")).toBool());
        QCOMPARE(applied.size(), 1);
        QCOMPARE(changed.count(), 1);
        QCOMPARE(changed.last().at(0).toString(), ConfigStore::hashOf(kConfig2));
        QCOMPARE(s.configHash(), ConfigStore::hashOf(kConfig2));

        // A hand edit, then ReloadConfig.
        writeFile(path, kConfig);
        r = obj(s.ReloadConfig());
        QVERIFY(r.value(QStringLiteral("ok")).toBool());
        QCOMPARE(applied.size(), 2);
        QCOMPARE(s.configHash(), ConfigStore::hashOf(kConfig));
        // A broken hand edit: rejected, the running config stays.
        writeFile(path, "{broken");
        r = obj(s.ReloadConfig());
        QVERIFY(!r.value(QStringLiteral("ok")).toBool());
        QCOMPARE(applied.size(), 2);
        QCOMPARE(rejected.count(), 1);
        QCOMPARE(s.configHash(), ConfigStore::hashOf(kConfig));
        QVERIFY(!obj(s.GetStatus()).value(QStringLiteral("config")).toObject().value(QStringLiteral("error")).toString().isEmpty());

        // The daemon refuses to apply: reported, not silently ignored.
        s.setConfigApplier([](const Config &) { return QStringLiteral("busy"); });
        writeFile(path, kConfig);
        r = obj(s.SetConfig(QString::fromUtf8(kConfig2), ConfigStore::hashOf(kConfig)));
        QCOMPARE(r.value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(), QStringLiteral("apply"));
    }

    // SetOption/GetOption/ListOptions: `control-surfaced set` for the Settings app.
    void serviceOptions()
    {
        QTemporaryDir t;
        const QString path = t.path() + QStringLiteral("/config.jsonc");
        const QByteArray text = "// mine\n{\n    \"profiles\": [],  // none yet\n    \"device\": { \"serial\": \"\", \"input\": \"auto\" }\n}\n";
        writeFile(path, text);
        SettingsService s(path);
        QList<Config> applied;
        s.setConfigApplier([&](const Config &c) {
            applied << c;
            return QString();
        });
        s.setConfigState(ConfigStore::hashOf(text), QString(), {});
        QSignalSpy changed(&s, &SettingsService::ConfigChanged);

        const QJsonObject list = obj(s.ListOptions());
        QVERIFY(list.value(QStringLiteral("ok")).toBool());
        QCOMPARE(list.value(QStringLiteral("options")).toArray().size(), settableOptions().size());
        const QJsonObject input = list.value(QStringLiteral("options")).toArray().first().toObject();
        QCOMPARE(input.value(QStringLiteral("key")).toString(), QStringLiteral("input"));
        QCOMPARE(input.value(QStringLiteral("labels")).toObject().value(QStringLiteral("evdev")).toString(), QStringLiteral("Keymap (compatible)"));

        QJsonObject r = obj(s.SetOption(QStringLiteral("input"), QStringLiteral("raw")));
        QVERIFY2(r.value(QStringLiteral("ok")).toBool(), qPrintable(QJsonDocument(r).toJson()));
        QVERIFY(r.value(QStringLiteral("changed")).toBool());
        QCOMPARE(applied.size(), 1);
        QCOMPARE(applied.last().device.input, QStringLiteral("raw"));
        QCOMPARE(changed.count(), 1);
        QCOMPARE(s.configHash(), r.value(QStringLiteral("hash")).toString());
        QByteArray now;
        {
            QFile f(path);
            QVERIFY(f.open(QIODevice::ReadOnly));
            now = f.readAll();
        }
        QCOMPARE(now, QByteArray(text).replace("\"auto\"", "\"raw\""));  // comments kept
        QVERIFY(QFile::exists(path + QStringLiteral(".bak")));

        r = obj(s.SetOption(QStringLiteral("cheatsheet.opacity"), QStringLiteral("0.5")));
        QVERIFY(r.value(QStringLiteral("ok")).toBool());
        QCOMPARE(applied.last().cheatsheet.opacity, 0.5);
        QCOMPARE(obj(s.GetOption(QStringLiteral("cheatsheet.opacity"))).value(QStringLiteral("value")).toDouble(), 0.5);
        QCOMPARE(obj(s.GetOption(QStringLiteral("input"))).value(QStringLiteral("effective")).toString(), QStringLiteral("raw"));

        r = obj(s.SetOption(QStringLiteral("input"), QStringLiteral("fast")));
        QCOMPARE(r.value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(), QStringLiteral("invalid-value"));
        r = obj(s.SetOption(QStringLiteral("profiles"), QStringLiteral("[]")));
        QCOMPARE(r.value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(), QStringLiteral("unknown-option"));
        QCOMPARE(applied.size(), 2);

        // Written, but the daemon refuses it: reported.
        s.setConfigApplier([](const Config &) { return QStringLiteral("busy"); });
        r = obj(s.SetOption(QStringLiteral("input"), QStringLiteral("evdev")));
        QVERIFY(!r.value(QStringLiteral("ok")).toBool());
        QCOMPARE(r.value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(), QStringLiteral("apply"));
    }

    void monitorFormat()
    {
        QCOMPARE(InputMonitor::jsonLine(QStringLiteral("key7"), QStringLiteral("press"), 0, 5), QStringLiteral(R"({"delta":0,"event":"press","ms":5,"slot":"key7"})"));
        QCOMPARE(InputMonitor::textLine(QStringLiteral("knob2"), QStringLiteral("ccw"), -1), QStringLiteral("knob2 ccw -1"));
        QCOMPARE(InputMonitor::textLine(QStringLiteral("key1"), QStringLiteral("release"), 0), QStringLiteral("key1 release"));
    }

    // --------------------------------------------------- service, private bus
    void serviceOverBus()
    {
        if (!privateBus()) {
            QSKIP("runs only on a private bus: ctest starts it under dbus-run-session");
        }
        QTemporaryDir t;
        const QString path = t.path() + QStringLiteral("/config.jsonc");
        writeFile(path, kConfig);
        QDBusConnection srv = bus(QStringLiteral("srv1"));
        QDBusConnection cli = bus(QStringLiteral("cli1"));
        {
            SettingsService s(path);
            QString err;
            QVERIFY2(s.registerOn(srv, true, &err), qPrintable(err));
            s.setConfigState(ConfigStore::hashOf(kConfig), QString(), {});
            Receiver r(cli);

            const QDBusMessage all = r.call(QStringLiteral("GetAll"), {QLatin1String(SettingsService::kInterface)}, QLatin1String(SettingsService::kPath),
                                            QStringLiteral("org.freedesktop.DBus.Properties"));
            QVERIFY2(all.type() == QDBusMessage::ReplyMessage, qPrintable(all.errorMessage()));
            QVariantMap props = r.props();
            QCOMPARE(props.value(QStringLiteral("ApiVersion")).toUInt(), 1u);
            QCOMPARE(props.value(QStringLiteral("DevicePresent")).toBool(), false);
            QCOMPARE(props.value(QStringLiteral("Identifying")).toBool(), false);

            DeviceState d;
            d.present = true;
            d.usb = usb(QStringLiteral("1189"), QStringLiteral("8890"), QStringLiteral("wch.cn"), QStringLiteral("CH552"), QStringLiteral("0100"), 4);
            d.firmware = classifyFirmware(d.usb);
            s.setDevice(d);
            QTRY_COMPARE(r.devices, QStringList{QStringLiteral("1 stock")});
            QTRY_COMPARE(r.changedProps.value(QStringLiteral("FirmwareType")).toString(), QStringLiteral("stock"));
            QCOMPARE(r.props().value(QStringLiteral("DevicePresent")).toBool(), true);
            s.setDevice(d);  // no change, no signal
            QTest::qWait(50);
            QCOMPARE(r.devices.size(), 1);

            // Live input reaches the client; identify suppresses.
            s.filterPadEvent(PadEvent{QStringLiteral("key7"), PadEvent::KeyDown, 0, 0});
            QTRY_COMPARE(r.inputs, QStringList{QStringLiteral("key7 press 0")});
            QCOMPARE(r.call(QStringLiteral("SetIdentify"), {true}).type(), QDBusMessage::ReplyMessage);
            QVERIFY(s.identifyActive());
            QTRY_COMPARE(r.identify, QList<bool>{true});
            QVERIFY(s.filterPadEvent(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, 1, 0}));
            QTRY_COMPARE(r.inputs.size(), 2);
            QCOMPARE(r.inputs.last(), QStringLiteral("knob1 cw 1"));
            r.call(QStringLiteral("SetIdentify"), {false});
            QVERIFY(!s.identifyActive());
            QTRY_COMPARE(r.identify, (QList<bool>{true, false}));

            // The caller leaving the bus ends identify.
            {
                QDBusConnection gone = bus(QStringLiteral("gone"));
                auto m = QDBusMessage::createMethodCall(QLatin1String(SettingsService::kService), QLatin1String(SettingsService::kPath),
                                                        QLatin1String(SettingsService::kInterface), QStringLiteral("SetIdentify"));
                m << true;
                QCOMPARE(gone.call(m, QDBus::BlockWithGui, 3000).type(), QDBusMessage::ReplyMessage);
                QVERIFY(s.identifyActive());
                QDBusConnection::disconnectFromBus(QStringLiteral("gone"));
            }
            QTRY_VERIFY(!s.identifyActive());

            // Config over the bus.
            const QDBusMessage gc = r.call(QStringLiteral("GetConfig"));
            QCOMPARE(gc.arguments().size(), 3);
            QCOMPARE(gc.arguments().at(0).toString().toUtf8(), kConfig);
            QCOMPARE(gc.arguments().at(2).toString(), ConfigStore::hashOf(kConfig));
            QJsonObject res = r.json(QStringLiteral("SetConfig"), {QString::fromUtf8(kConfig2), gc.arguments().at(2).toString()});
            QVERIFY(res.value(QStringLiteral("ok")).toBool());
            QTRY_COMPARE(r.configs, QStringList{ConfigStore::hashOf(kConfig2)});
            QTRY_COMPARE(r.changedProps.value(QStringLiteral("ConfigHash")).toString(), ConfigStore::hashOf(kConfig2));
            res = r.json(QStringLiteral("ValidateConfig"), {QStringLiteral("{")});
            QVERIFY(!res.value(QStringLiteral("ok")).toBool());

            // Plugins and firmware status.
            s.setPlugins({PluginInfo{QStringLiteral("kdenlive"), QStringLiteral("Kdenlive"), QStringLiteral("api"), QStringLiteral("absent"), {}, {}, {}}});
            QTRY_COMPARE(r.plugins, 1);
            QCOMPARE(r.json(QStringLiteral("ListPlugins")).value(QStringLiteral("plugins")).toArray().size(), 1);

            // A dry-run flash: the reply carries the job id before any progress.
            FlashFixture f;
            s.setFlashSettings(f.settings);
            writeFile(f.dir.path() + QStringLiteral("/fw/pad.json"), QJsonDocument(QJsonObject{{QStringLiteral("version"), QStringLiteral("2.0.0")}, {QStringLiteral("sha256"), f.imageSha}}).toJson());
            const QJsonObject fw = r.json(QStringLiteral("GetFirmwareStatus"));
            const QJsonObject img = fw.value(QStringLiteral("images")).toArray().first().toObject();
            QCOMPARE(img.value(QStringLiteral("sha256")).toString(), f.imageSha);
            QCOMPARE(img.value(QStringLiteral("meta")).toObject().value(QStringLiteral("version")).toString(), QStringLiteral("2.0.0"));
            QCOMPARE(img.value(QStringLiteral("metaMatches")).toBool(), true);
            QCOMPARE(fw.value(QStringLiteral("flash")).toObject().value(QStringLiteral("toolFound")).toBool(), true);
            res = r.json(QStringLiteral("StartFlash"), {f.image, f.imageSha, true});
            QVERIFY(res.value(QStringLiteral("ok")).toBool());
            const QString job = res.value(QStringLiteral("jobId")).toString();
            QTRY_COMPARE(r.flash.size(), 7);
            QCOMPARE(r.flash.first(), job + QStringLiteral(":checking"));
            QCOMPARE(r.flash.last(), job + QStringLiteral(":done"));

            // A real job: release requested, busy for a second caller, cancellable.
            QSignalSpy rel(&s, &SettingsService::releaseDeviceRequested);
            QSignalSpy acq(&s, &SettingsService::reacquireDeviceRequested);
            s.setJobSetup([](FlashJob *j) { j->setProbe([] { return QList<UsbDeviceInfo>{}; }); });
            res = r.json(QStringLiteral("StartFlash"), {f.image, f.imageSha, false});
            QVERIFY(res.value(QStringLiteral("ok")).toBool());
            QTRY_COMPARE(rel.count(), 1);
            QCOMPARE(r.json(QStringLiteral("StartFlash"), {f.image, f.imageSha, true}).value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(),
                     QStringLiteral("busy"));
            QCOMPARE(r.call(QStringLiteral("CancelFlash"), {QStringLiteral("nope")}).arguments().value(0).toBool(), false);
            QCOMPARE(r.call(QStringLiteral("CancelFlash"), {res.value(QStringLiteral("jobId")).toString()}).arguments().value(0).toBool(), true);
            QCOMPARE(acq.count(), 1);
            QCOMPARE(r.json(QStringLiteral("GetStatus")).value(QStringLiteral("flash")).toObject().value(QStringLiteral("phase")).toString(), QStringLiteral("cancelled"));

            // A second daemon cannot take the name.
            SettingsService other(path);
            QDBusConnection srv2 = bus(QStringLiteral("srv2"));
            QString err2;
            QVERIFY(!other.registerOn(srv2, true, &err2));
            QVERIFY(err2.contains(QStringLiteral("another daemon")));
            srv.unregisterService(QLatin1String(SettingsService::kService));
            srv.unregisterObject(QLatin1String(SettingsService::kPath));
        }
        QDBusConnection::disconnectFromBus(QStringLiteral("srv1"));
        QDBusConnection::disconnectFromBus(QStringLiteral("srv2"));
        QDBusConnection::disconnectFromBus(QStringLiteral("cli1"));
    }

    // -------------------------------------------------------------- the mock
    void mockOverBus()
    {
        if (!privateBus()) {
            QSKIP("runs only on a private bus: ctest starts it under dbus-run-session");
        }
        FlashFixture f;
        const QString cfgPath = f.dir.path() + QStringLiteral("/config.jsonc");
        writeFile(cfgPath, kConfig);
        QDBusConnection srv = bus(QStringLiteral("msrv"));
        QDBusConnection cli = bus(QStringLiteral("mcli"));
        {
            MockSurface::Options o;
            o.configPath = cfgPath;
            o.firmwareDir = f.dir.path() + QStringLiteral("/fw");
            o.flashStepMs = 20;
            MockSurface mock(o);
            QString err;
            QVERIFY2(mock.registerOn(srv, true, &err), qPrintable(err));
            Receiver r(cli);

            QCOMPARE(r.props().value(QStringLiteral("Mode")).toString(), QStringLiteral("mock"));
            QCOMPARE(r.props().value(QStringLiteral("FirmwareType")).toString(), QStringLiteral("control-surface"));
            QCOMPARE(r.json(QStringLiteral("GetLayout")).value(QStringLiteral("layout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));

            // Input is dispatched through a real engine into recorded keys.
            QCOMPARE(r.mock(QStringLiteral("Press"), {QStringLiteral("key1")}).type(), QDBusMessage::ReplyMessage);
            QTRY_COMPARE(r.inputs, (QStringList{QStringLiteral("key1 press 0"), QStringLiteral("key1 release 0")}));
            QCOMPARE(r.mock(QStringLiteral("TakeKeys")).arguments().value(0).toStringList(), QStringList{QStringLiteral("F13")});
            QTRY_VERIFY(!r.dispatched.isEmpty());
            r.mock(QStringLiteral("Turn"), {QStringLiteral("knob1"), 2});
            QStringList turned;  // knob taps are paced by the engine (keyRateHz)
            QTRY_VERIFY((turned += r.mock(QStringLiteral("TakeKeys")).arguments().value(0).toStringList()).size() >= 2);
            QCOMPARE(turned, (QStringList{QStringLiteral("F16"), QStringLiteral("F16")}));
            QTRY_COMPARE(r.inputs.size(), 4);
            QCOMPARE(r.inputs.last(), QStringLiteral("knob1 cw 1"));

            // Identify suppresses dispatch.
            r.call(QStringLiteral("SetIdentify"), {true});
            r.mock(QStringLiteral("Press"), {QStringLiteral("key2")});
            QTRY_COMPARE(r.inputs.size(), 6);
            QVERIFY(r.mock(QStringLiteral("TakeKeys")).arguments().value(0).toStringList().isEmpty());
            r.call(QStringLiteral("SetIdentify"), {false});
            r.mock(QStringLiteral("Press"), {QStringLiteral("key2")});
            QCOMPARE(r.mock(QStringLiteral("TakeKeys")).arguments().value(0).toStringList(), QStringList{QStringLiteral("F15")});

            // Bad arguments are D-Bus errors.
            QCOMPARE(r.mock(QStringLiteral("Press"), {QStringLiteral("key99")}).type(), QDBusMessage::ErrorMessage);
            QCOMPARE(r.mock(QStringLiteral("Turn"), {QStringLiteral("knob1"), 0}).type(), QDBusMessage::ErrorMessage);
            QCOMPARE(r.mock(QStringLiteral("SetFlashOutcome"), {QStringLiteral("explode")}).type(), QDBusMessage::ErrorMessage);

            // Hot-plug for the bar icon.
            r.mock(QStringLiteral("Unplug"));
            QTRY_VERIFY(r.devices.contains(QStringLiteral("0 ")));
            r.mock(QStringLiteral("Plug"), {QStringLiteral("stock"), QString()});
            QTRY_VERIFY(r.devices.contains(QStringLiteral("1 stock")));

            // Plugin status.
            r.mock(QStringLiteral("SetPluginStatus"), {QStringLiteral("kdenlive"), QStringLiteral("available")});
            const QJsonArray plugins = r.json(QStringLiteral("ListPlugins")).value(QStringLiteral("plugins")).toArray();
            bool found = false;
            for (const auto &p : plugins) {
                found = found || (p.toObject().value(QStringLiteral("id")).toString() == QLatin1String("kdenlive")
                                  && p.toObject().value(QStringLiteral("status")).toString() == QLatin1String("available"));
            }
            QVERIFY(found);

            // The flash wizard, end to end, with simulated hardware.
            const QString image = f.image;
            QJsonObject res = r.json(QStringLiteral("StartFlash"), {image, f.imageSha, false});
            QVERIFY2(res.value(QStringLiteral("ok")).toBool(), qPrintable(QJsonDocument(res).toJson()));
            const QString job = res.value(QStringLiteral("jobId")).toString();
            QTRY_VERIFY(r.flash.contains(job + QStringLiteral(":waiting-bootloader")));
            r.mock(QStringLiteral("EnterBootloader"));
            QTRY_VERIFY(r.devices.contains(QStringLiteral("0 ")));
            QTRY_VERIFY_WITH_TIMEOUT(r.flash.contains(job + QStringLiteral(":done")), 5000);
            QCOMPARE(r.props().value(QStringLiteral("FirmwareType")).toString(), QStringLiteral("control-surface"));

            r.mock(QStringLiteral("SetFlashOutcome"), {QStringLiteral("tool-fails")});
            res = r.json(QStringLiteral("StartFlash"), {image, f.imageSha, false});
            const QString job2 = res.value(QStringLiteral("jobId")).toString();
            QTRY_VERIFY(r.flash.contains(job2 + QStringLiteral(":waiting-bootloader")));
            r.mock(QStringLiteral("EnterBootloader"));
            QTRY_VERIFY_WITH_TIMEOUT(r.flash.contains(job2 + QStringLiteral(":failed")), 5000);

            srv.unregisterService(QLatin1String(SettingsService::kService));
        }
        QDBusConnection::disconnectFromBus(QStringLiteral("msrv"));
        QDBusConnection::disconnectFromBus(QStringLiteral("mcli"));
    }

    void monitorFollowsBus()
    {
        if (!privateBus()) {
            QSKIP("runs only on a private bus: ctest starts it under dbus-run-session");
        }
        QTemporaryDir t;
        writeFile(t.path() + QStringLiteral("/c.jsonc"), kConfig);
        QDBusConnection srv = bus(QStringLiteral("monsrv"));
        QDBusConnection cli = bus(QStringLiteral("moncli"));
        InputMonitor early;
        QVERIFY(!early.attach(cli));  // nobody there yet
        {
            SettingsService s(t.path() + QStringLiteral("/c.jsonc"));
            QVERIFY(s.registerOn(srv, true, nullptr));
            InputMonitor mon;
            QVERIFY(mon.attach(cli));
            QSignalSpy in(&mon, &InputMonitor::input);
            QSignalSpy gone(&mon, &InputMonitor::daemonGone);
            s.filterPadEvent(PadEvent{QStringLiteral("key4"), PadEvent::KeyDown, 0, 0});
            QTRY_COMPARE(in.count(), 1);
            QCOMPARE(in.first(), (QVariantList{QStringLiteral("key4"), QStringLiteral("press"), 0}));
            srv.unregisterService(QLatin1String(SettingsService::kService));
            QTRY_COMPARE(gone.count(), 1);
        }
        {
            // The daemon restarts: the same monitor keeps following it.
            InputMonitor mon;
            SettingsService s(t.path() + QStringLiteral("/c.jsonc"));
            QVERIFY(s.registerOn(srv, true, nullptr));
            QVERIFY(mon.attach(cli));
            QSignalSpy in(&mon, &InputMonitor::input);
            QSignalSpy gone(&mon, &InputMonitor::daemonGone);
            QSignalSpy back2(&mon, &InputMonitor::daemonBack);
            srv.unregisterService(QLatin1String(SettingsService::kService));
            QTRY_COMPARE(gone.count(), 1);
            SettingsService again(t.path() + QStringLiteral("/c.jsonc"));
            QDBusConnection srv2 = bus(QStringLiteral("monsrv2"));
            QVERIFY(again.registerOn(srv2, true, nullptr));
            QTRY_COMPARE(back2.count(), 1);
            again.filterPadEvent(PadEvent{QStringLiteral("key9"), PadEvent::KeyUp, 0, 0});
            QTRY_COMPARE(in.count(), 1);
            QCOMPARE(in.first(), (QVariantList{QStringLiteral("key9"), QStringLiteral("release"), 0}));
            srv2.unregisterService(QLatin1String(SettingsService::kService));
        }
        QDBusConnection::disconnectFromBus(QStringLiteral("monsrv"));
        QDBusConnection::disconnectFromBus(QStringLiteral("monsrv2"));
        QDBusConnection::disconnectFromBus(QStringLiteral("moncli"));
    }

    void cheatsheetOverBus()
    {
        if (!privateBus()) {
            QSKIP("runs only on a private bus: ctest starts it under dbus-run-session");
        }
        QTemporaryDir t;
        const QString cfgPath = t.path() + QStringLiteral("/config.jsonc");
        writeFile(cfgPath, R"({"cheatsheet":{"autoHideMs":0,"opacity":0.6,"position":"top"},"profiles":[
            {"name":"Kdenlive","match":{"class":"^org\\.kde\\.kdenlive"},"kdenlive":true,
             "layers":[{"name":"Wheels","when":{"colorWheels":true},"bindings":{"key2":{"request":"colorwheel.reset","params":{"wheel":"lift"}}}}],
             "bindings":{"key2":{"action":"mark_in"}}},
            {"name":"global","bindings":{"key1":{"cheatsheet":"toggle"},"key3":{"cheatsheet":"hold"},"key2":"ctrl+z"}}]})");
        QDBusConnection srv = bus(QStringLiteral("csrv"));
        QDBusConnection cli = bus(QStringLiteral("ccli"));
        {
            MockSurface::Options o;
            o.configPath = cfgPath;
            o.firmwareDir = t.path() + QStringLiteral("/fw");
            MockSurface mock(o);
            QString err;
            QVERIFY2(mock.registerOn(srv, true, &err), qPrintable(err));
            Receiver r(cli);

            QCOMPARE(r.props().value(QStringLiteral("CheatsheetVisible")).toBool(), false);
            QJsonObject c = r.json(QStringLiteral("GetCheatsheet"));
            QCOMPARE(c.value(QStringLiteral("ok")).toBool(), true);
            QCOMPARE(c.value(QStringLiteral("visible")).toBool(), false);
            QCOMPARE(c.value(QStringLiteral("profile")).toString(), QStringLiteral("global"));
            QCOMPARE(c.value(QStringLiteral("options")).toObject().value(QStringLiteral("position")).toString(), QStringLiteral("top"));
            QCOMPARE(c.value(QStringLiteral("layout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));

            // The mapped key shows it; the overlay gets visibility and content.
            r.mock(QStringLiteral("Press"), {QStringLiteral("key1")});
            QTRY_COMPARE(r.sheetVisible, QList<bool>{true});
            QTRY_COMPARE(r.sheets.size(), 1);
            QCOMPARE(obj(r.sheets.last()).value(QStringLiteral("visible")).toBool(), true);
            QTRY_COMPARE(r.changedProps.value(QStringLiteral("CheatsheetVisible")).toBool(), true);
            QVERIFY(r.mock(QStringLiteral("TakeKeys")).arguments().value(0).toStringList().isEmpty());  // nothing typed

            // Focus Kdenlive with the wheels open: new content while shown.
            r.mock(QStringLiteral("SetKdenliveState"), {QStringLiteral("available")});
            r.mock(QStringLiteral("SetKdenliveContext"), {QStringLiteral(R"({"colorWheels": true, "focus": "effectStack"})")});
            r.mock(QStringLiteral("Focus"), {QStringLiteral("org.kde.kdenlive"), QStringLiteral("x")});
            QTRY_VERIFY(!r.sheets.isEmpty() && obj(r.sheets.last()).value(QStringLiteral("title")).toString() == QStringLiteral("Kdenlive · Wheels"));
            c = obj(r.sheets.last());
            QString key2;
            for (const auto &k : c.value(QStringLiteral("keys")).toArray()) {
                if (k.toObject().value(QStringLiteral("control")).toString() == QLatin1String("key2")) {
                    key2 = k.toObject().value(QStringLiteral("label")).toString();
                }
            }
            QCOMPARE(key2, QStringLiteral("Reset lift"));
            // The wheels close.
            const int before = r.sheets.size();
            r.mock(QStringLiteral("SetKdenliveContext"), {QStringLiteral(R"({"focus": "timeline"})")});
            QTRY_COMPARE(r.sheets.size(), before + 1);
            QCOMPARE(obj(r.sheets.last()).value(QStringLiteral("title")).toString(), QStringLiteral("Kdenlive"));
            QCOMPARE(r.mock(QStringLiteral("SetKdenliveState"), {QStringLiteral("gone")}).type(), QDBusMessage::ErrorMessage);
            QCOMPARE(r.mock(QStringLiteral("SetKdenliveContext"), {QStringLiteral("[1]")}).type(), QDBusMessage::ErrorMessage);

            // Methods.
            r.call(QStringLiteral("HideCheatsheet"));
            QTRY_COMPARE(r.sheetVisible, (QList<bool>{true, false}));
            QCOMPARE(r.json(QStringLiteral("GetStatus")).value(QStringLiteral("cheatsheet")).toObject().value(QStringLiteral("visible")).toBool(), false);
            r.call(QStringLiteral("ToggleCheatsheet"));
            QTRY_COMPARE(r.sheetVisible.size(), 3);
            r.call(QStringLiteral("ShowCheatsheet"));
            QTest::qWait(50);
            QCOMPARE(r.sheetVisible.size(), 3);  // already shown
            // Unplugging hides it (a held key can no longer be released).
            r.mock(QStringLiteral("Unplug"));
            QTRY_COMPARE(r.sheetVisible.size(), 4);
            QCOMPARE(r.sheetVisible.last(), false);
            r.mock(QStringLiteral("Plug"), {QStringLiteral("control-surface"), QString()});
            // Hold.
            r.mock(QStringLiteral("Hold"), {QStringLiteral("key3")});
            QTRY_COMPARE(r.sheetVisible.size(), 5);
            r.mock(QStringLiteral("Release"), {QStringLiteral("key3")});
            QTRY_COMPARE(r.sheetVisible.size(), 6);
            QCOMPARE(r.sheetVisible.last(), false);

            // Previews for editors.
            c = r.json(QStringLiteral("GetCheatsheetFor"), {QStringLiteral("org.kde.kdenlive"), QString(), QStringLiteral(R"({"colorWheels":true})")});
            QCOMPARE(c.value(QStringLiteral("preview")).toBool(), true);
            QCOMPARE(c.value(QStringLiteral("title")).toString(), QStringLiteral("Kdenlive · Wheels"));
            c = r.json(QStringLiteral("GetCheatsheetFor"), {QStringLiteral("brave-browser"), QString(), QString()});
            QCOMPARE(c.value(QStringLiteral("profile")).toString(), QStringLiteral("global"));
            c = r.json(QStringLiteral("GetCheatsheetFor"), {QStringLiteral("x"), QString(), QStringLiteral("not json")});
            QCOMPARE(c.value(QStringLiteral("error")).toObject().value(QStringLiteral("code")).toString(), QStringLiteral("invalid-arguments"));
            QCOMPARE(r.props().value(QStringLiteral("CheatsheetVisible")).toBool(), false);  // previews never show it
            srv.unregisterService(QLatin1String(SettingsService::kService));
        }
        QDBusConnection::disconnectFromBus(QStringLiteral("csrv"));
        QDBusConnection::disconnectFromBus(QStringLiteral("ccli"));
    }

    void mockPushesToEww()
    {
        // mock-control-surfaced --eww-window pad-cheatsheet: the same push as the
        // daemon, so the overlay can be built against the mock.
        QTemporaryDir t;
        QVERIFY(ewwmock::install(t.path()));
        const QString log = t.path() + QStringLiteral("/calls.log");
        const QByteArray oldPath = qgetenv("PATH");
        qputenv("EWW_MOCK_LOG", log.toUtf8());
        qputenv("PATH", (t.path() + QLatin1Char(':')).toUtf8() + oldPath);
        const QString cfgPath = t.path() + QStringLiteral("/config.jsonc");
        writeFile(cfgPath, R"({"cheatsheet": {"position": "bottom-left"}, "profiles": [{"name": "global", "bindings": {"key1": {"cheatsheet": "toggle"}}}]})");
        {
            MockSurface::Options o;
            o.configPath = cfgPath;
            o.firmwareDir = t.path() + QStringLiteral("/fw");
            o.eww.enabled = true;
            o.eww.window = QStringLiteral("pad-cheatsheet");
            MockSurface mock(o);
            QTRY_COMPARE(ewwmock::summaries(log), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
            mock.Press(QStringLiteral("key1"));
            QTRY_COMPARE(ewwmock::summaries(log).mid(2), (QStringList{QStringLiteral("update pad_sheet visible=true"), QStringLiteral("open pad-cheatsheet @bottom left")}));
            QTRY_VERIFY(!mock.eww().isBusy());
            const QJsonObject eww = obj(mock.settings().GetStatus()).value(QStringLiteral("cheatsheet")).toObject().value(QStringLiteral("eww")).toObject();
            QCOMPARE(eww.value(QStringLiteral("enabled")).toBool(), true);
            QCOMPARE(eww.value(QStringLiteral("calls")).toInt(), 4);
            // Shown by a toggle key with autoHideMs unset: it goes by itself after 8 s.
            QVERIFY(mock.cheatsheet().autoHideActive());
            QCOMPARE(mock.cheatsheet().autoHideIntervalMs(), 8000);
            // A click on the overlay: HideCheatsheet, idempotent, and the
            // window is closed every time.
            QSignalSpy visible(&mock.settings(), &SettingsService::CheatsheetVisibilityChanged);
            mock.settings().HideCheatsheet();
            QTRY_VERIFY(!mock.eww().isBusy());
            QCOMPARE(ewwmock::summaries(log).mid(4), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
            mock.settings().HideCheatsheet();
            QTRY_VERIFY(!mock.eww().isBusy());
            QCOMPARE(ewwmock::summaries(log).mid(6), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
            QCOMPARE(visible.count(), 1);  // one real change
            QVERIFY(!mock.cheatsheet().isVisible());
            // ShowCheatsheet (the bar, Settings) also gets the default auto-hide.
            mock.settings().ShowCheatsheet();
            QVERIFY(mock.cheatsheet().autoHideActive());
            QTRY_VERIFY(!mock.eww().isBusy());
            mock.settings().HideCheatsheet();
            QTRY_VERIFY(!mock.eww().isBusy());
            QCOMPARE(ewwmock::summaries(log).size(), 12);
        }
        // Quitting the mock: already hidden, nothing more to do.
        QCOMPARE(ewwmock::summaries(log).size(), 12);
        QCOMPARE(ewwmock::summaries(log).mid(10), (QStringList{QStringLiteral("close pad-cheatsheet"), QStringLiteral("update pad_sheet visible=false")}));
        qputenv("PATH", oldPath);
    }
};

QTEST_GUILESS_MAIN(TestSettings)
#include "tst_settings.moc"
