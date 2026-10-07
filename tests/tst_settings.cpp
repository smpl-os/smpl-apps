// SPDX-License-Identifier: GPL-2.0-or-later
// Settings API (org.smplos.ControlSurface1), its building blocks and the mock.
// Bus tests run only on a PRIVATE bus (ctest starts this under dbus-run-session).
#include "boardprofile.h"
#include "configstore.h"
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

    QStringList inputs, devices, configs, rejected, flash, dispatched;
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
        for (const char *n : {"srv1", "srv2", "cli1", "msrv", "mcli", "monsrv", "moncli", "gone"}) {
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
        j->start();
        QCOMPARE(rel.count(), 1);
        QCOMPARE(j->phase(), QStringLiteral("waiting-bootloader"));
        QTest::qWait(60);
        QCOMPARE(j->phase(), QStringLiteral("waiting-bootloader"));  // the old session is ignored
        f.devices = {usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 5),
                     usb(QStringLiteral("4348"), QStringLiteral("55e0"), {}, {}, QStringLiteral("0240"), 1, 6)};
        QTRY_COMPARE(j->phase(), QStringLiteral("flashing"));
        QCOMPARE(f.ranImage, f.image);
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
        s.setDevice(d);
        const QJsonObject layout = obj(s.GetLayout()).value(QStringLiteral("layout")).toObject();
        QCOMPARE(layout.value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));
        QCOMPARE(layout.value(QStringLiteral("source")).toString(), QStringLiteral("firmware"));
        QCOMPARE(obj(s.GetDevice()).value(QStringLiteral("firmware")).toObject().value(QStringLiteral("type")).toString(), QStringLiteral("control-surface"));
        QVERIFY(obj(s.ListBoardProfiles()).value(QStringLiteral("profiles")).toArray().size() >= 5);
        const QJsonObject st = obj(s.GetStatus());
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
        QDBusConnection::disconnectFromBus(QStringLiteral("monsrv"));
        QDBusConnection::disconnectFromBus(QStringLiteral("moncli"));
    }
};

QTEST_GUILESS_MAIN(TestSettings)
#include "tst_settings.moc"
