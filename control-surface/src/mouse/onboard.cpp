// SPDX-License-Identifier: GPL-2.0-or-later
#include "onboard.h"
#include "mapping.h"
#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>
#include <QTextStream>
#include <QThread>
#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <linux/hidraw.h>
#include <linux/input.h>
#include <sys/file.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

namespace cs::mouse::onboard {
namespace {
constexpr int ReportLength = 91;
constexpr quint16 GetButton = 0x028c;

bool allowedRead(quint16 command) {
    return command == 0x0081 || command == 0x0082 || command == 0x058a || command == 0x0580 || command == GetButton;
}

quint8 byte(const QByteArray &value, int index) { return quint8(value.at(index)); }

quint8 checksum(const QByteArray &report) {
    quint8 value = 0;
    for (int index = 3; index <= 88; ++index)
        value ^= byte(report, index);
    return value;
}

std::optional<QByteArray> readFile(const QString &path, QString *error) {
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        *error = QStringLiteral("Read %1: %2").arg(path, file.errorString());
        return std::nullopt;
    }
    const auto bytes = file.read(65537);
    if (file.error() != QFileDevice::NoError) {
        *error = QStringLiteral("Read %1: %2").arg(path, file.errorString());
        return std::nullopt;
    }
    if (bytes.size() > 65536) {
        *error = QStringLiteral("Read %1: file exceeds the 64 KiB limit").arg(path);
        return std::nullopt;
    }
    return bytes;
}

bool hex(const QString &value, int bytes) {
    return value.size() == bytes * 2 && QRegularExpression(QStringLiteral("^[0-9a-fA-F]+$")).match(value).hasMatch();
}

QByteArray buttonPrefix(const QString &address) {
    const auto fields = address.split('/');
    QByteArray prefix(3, 0);
    prefix[0] = 1;
    prefix[1] = char(fields[0].toInt(nullptr, 16));
    prefix[2] = char(fields[1].toInt());
    return prefix;
}

QByteArray originalDpi(int button) {
    return QByteArray::fromHex(button == 0x0b ? "06010100000000" : "06010200000000");
}

QByteArray mappedDpi(int button) {
    return QByteArray::fromHex(button == 0x0b ? "02020072000000" : "02020073000000");
}

QByteArray makeReport(quint16 command, const QByteArray &arguments, int responseSize) {
    QByteArray report(ReportLength, 0);
    report[2] = 0x1f;
    report[6] = char(responseSize);
    report[7] = char(command >> 8);
    report[8] = char(command & 0xff);
    report.replace(9, arguments.size(), arguments);
    report[89] = char(checksum(report));
    return report;
}

bool validBackup(const QJsonObject &value, QString *error) {
    const auto device = value.value("device").toObject();
    const auto assignments = value.value("assignments").toObject();
    const auto evidence = value.value("readEvidence").toArray();
    if (value.value("schemaVersion").toDouble() != 2 || value.value("profile").toDouble() != 1 ||
        value.value("scope") != "normal-layer-only" || value.value("hypershiftVerified") != false ||
        value.value("verifiedReads").toDouble() != 2 ||
        device.value("vendor") != "1532" || device.value("product") != "00b4" ||
        !hex(device.value("serialSha256").toString(), 32) || !hex(device.value("firmware").toString(), 4) ||
        assignments.size() != assignmentAddresses().size() / 2 || evidence.size() != 2 ||
        !evidence[0].isObject() || evidence[0] != evidence[1] ||
        evidence[0].toObject().size() != assignmentAddresses().size()) {
        *error = QStringLiteral("Invalid or incomplete Naga onboard backup");
        return false;
    }
    const auto replies = evidence[0].toObject();
    for (const auto &address : assignmentAddresses()) {
        const auto raw = replies.value(address).toString();
        if (!hex(raw, 10)) {
            *error = QStringLiteral("Missing raw read evidence for %1").arg(address);
            return false;
        }
        if (!address.endsWith("/0"))
            continue;
        const auto text = assignments.value(address).toString();
        if (!hex(text, 7) || byte(QByteArray::fromHex(text.toLatin1()), 1) > 5 ||
            QByteArray::fromHex(raw.toLatin1()) != buttonPrefix(address) + QByteArray::fromHex(text.toLatin1())) {
            *error = QStringLiteral("Invalid raw button function for %1").arg(address);
            return false;
        }
    }
    return true;
}

class Channel {
public:
    ~Channel() { if (m_fd >= 0) ::close(m_fd); }

    bool open(QString *error) {
        QStringList nodes;
        QString expectedDevice;
        const QDir directory(QStringLiteral("/sys/class/hidraw"));
        for (const auto &entry : directory.entryList({"hidraw*"}, QDir::Dirs | QDir::System)) {
            const QString device = directory.filePath(entry + "/device");
            auto uevent = readFile(device + "/uevent", error);
            if (!uevent)
                return false;
            if (!uevent->split('\n').contains("HID_ID=0003:00001532:000000B4"))
                continue;
            const QString interface = QFileInfo(device).canonicalFilePath() + "/../bInterfaceNumber";
            auto number = readFile(interface, error);
            if (!number)
                return false;
            if (number->trimmed() == "00") {
                nodes << "/dev/" + entry;
                expectedDevice = QFileInfo(device).canonicalFilePath();
            }
        }
        if (nodes.size() != 1) {
            *error = QStringLiteral("Expected exactly one 1532:00b4 USB interface-0 hidraw control channel; found %1").arg(nodes.size());
            return false;
        }
        m_fd = ::open(QFile::encodeName(nodes.first()).constData(), O_RDWR | O_CLOEXEC | O_NOFOLLOW);
        if (m_fd < 0) {
            *error = QStringLiteral("Open %1: %2. Use a narrowly scoped pkexec read-backup invocation; do not broaden device permissions.")
                         .arg(nodes.first(), QString::fromLocal8Bit(strerror(errno)));
            return false;
        }
        struct stat info {};
        hidraw_devinfo identity {};
        if (fstat(m_fd, &info) != 0 || !S_ISCHR(info.st_mode) || ioctl(m_fd, HIDIOCGRAWINFO, &identity) < 0 ||
            identity.bustype != BUS_USB || quint16(identity.vendor) != Vendor || quint16(identity.product) != Product) {
            *error = QStringLiteral("Opened hidraw node is not the expected USB Razer Naga receiver");
            return false;
        }
        const QString openedDevice = QFileInfo(QStringLiteral("/sys/dev/char/%1:%2/device")
                                                   .arg(major(info.st_rdev)).arg(minor(info.st_rdev))).canonicalFilePath();
        if (expectedDevice.isEmpty() || openedDevice != expectedDevice) {
            *error = QStringLiteral("Hidraw interface changed during discovery; no vendor query sent");
            return false;
        }
        if (flock(m_fd, LOCK_EX | LOCK_NB) != 0) {
            *error = QStringLiteral("The Naga control channel is already in use; no settings changed");
            return false;
        }
        return true;
    }

    std::optional<QByteArray> read(quint16 command, const QByteArray &arguments, int size, QString *error) {
        const auto request = readReport(command, arguments, size);
        if (request.isEmpty()) {
            *error = QStringLiteral("Refused non-read or malformed vendor request");
            return std::nullopt;
        }
        return transact(command, request, size, error);
    }

    bool writeDpi(int button, const QByteArray &function, QString *error) {
        const auto request = dpiWriteReport(button, function);
        if (request.isEmpty()) {
            *error = QStringLiteral("Refused a write outside the two approved normal-layer DPI assignments");
            return false;
        }
        return transact(0x020c, request, 10, error).has_value();
    }

private:
    std::optional<QByteArray> transact(quint16 command, const QByteArray &request, int size, QString *error) {
        const int sent = ioctl(m_fd, HIDIOCSFEATURE(ReportLength), request.constData());
        if (sent != ReportLength) {
            *error = QStringLiteral("Send vendor command 0x%1: %2")
                         .arg(command, 4, 16, QLatin1Char('0'))
                         .arg(sent < 0 ? QString::fromLocal8Bit(strerror(errno))
                                       : QStringLiteral("short transfer (%1 of %2 bytes)").arg(sent).arg(ReportLength));
            return std::nullopt;
        }
        QElapsedTimer elapsed;
        elapsed.start();
        while (elapsed.elapsed() < 1000) {
            QThread::msleep(5);
            QByteArray response(ReportLength, 0);
            const int received = ioctl(m_fd, HIDIOCGFEATURE(ReportLength), response.data());
            if (received != ReportLength) {
                *error = QStringLiteral("Receive vendor command 0x%1: %2")
                             .arg(command, 4, 16, QLatin1Char('0'))
                             .arg(received < 0 ? QString::fromLocal8Bit(strerror(errno))
                                               : QStringLiteral("short transfer (%1 of %2 bytes)").arg(received).arg(ReportLength));
                return std::nullopt;
            }
            auto reply = decodeReply(response, command, size);
            if (reply.state == ReplyState::Busy || reply.state == ReplyState::Unrelated)
                continue;
            if (reply.state == ReplyState::Ready)
                return reply.arguments;
            *error = reply.error;
            return std::nullopt;
        }
        *error = QStringLiteral("Timed out waiting for a matching vendor reply; stop other Razer configuration clients");
        return std::nullopt;
    }

    int m_fd = -1;
};
} // namespace

QByteArray readReport(quint16 command, const QByteArray &arguments, int responseSize) {
    if (!allowedRead(command) || responseSize < 1 || responseSize > 80 || arguments.size() > responseSize)
        return {};
    return makeReport(command, arguments, responseSize);
}

QByteArray dpiWriteReport(int button, const QByteArray &function) {
    if ((button != 0x0b && button != 0x0c) || (function != originalDpi(button) && function != mappedDpi(button)))
        return {};
    const auto address = QStringLiteral("%1/0").arg(button, 2, 16, QLatin1Char('0'));
    return makeReport(0x020c, buttonPrefix(address) + function, 10);
}

Reply decodeReply(const QByteArray &report, quint16 command, int responseSize) {
    if (report.size() != ReportLength || report[0] != 0 || report[3] != 0 || report[4] != 0 || report[5] != 0 ||
        report[90] != 0 || byte(report, 6) > 80 || checksum(report) != byte(report, 89))
        return {ReplyState::Error, {}, QStringLiteral("Malformed vendor reply (length, framing or checksum)")};
    if (byte(report, 2) != 0x1f || byte(report, 7) != command >> 8 || byte(report, 8) != (command & 0xff))
        return {ReplyState::Unrelated, {}, {}};
    if (byte(report, 1) == 1)
        return {ReplyState::Busy, {}, {}};
    if (byte(report, 1) != 2)
        return {ReplyState::Error, {}, QStringLiteral("Vendor command 0x%1 refused with status %2")
                     .arg(command, 4, 16, QLatin1Char('0')).arg(byte(report, 1))};
    if (byte(report, 6) != responseSize)
        return {ReplyState::Error, {}, QStringLiteral("Vendor reply has an unexpected argument length")};
    return {ReplyState::Ready, report.mid(9, responseSize), {}};
}

QStringList assignmentAddresses() {
    QList<int> buttons{1, 2, 3, 9, 10, 11, 12, 0x34, 0x35};
    for (int button = 0x40; button <= 0x4b; ++button)
        buttons << button;
    QStringList result;
    for (int layer : {0, 1})
        for (int button : buttons)
            result << QStringLiteral("%1/%2").arg(button, 2, 16, QLatin1Char('0')).arg(layer);
    return result;
}

std::optional<QJsonObject> probe(const Read &read, QString *error) {
    const auto firmware = read(0x0081, {}, 4, error);
    if (!firmware) return std::nullopt;
    if (firmware->size() != 4) {
        *error = QStringLiteral("Malformed firmware diagnostic readback");
        return std::nullopt;
    }
    QJsonArray queries;
    for (int pass = 1; pass <= 2; ++pass) {
        for (const auto &address : assignmentAddresses()) {
            const auto prefix = buttonPrefix(address);
            const auto args = read(GetButton, prefix, 10, error);
            if (!args) return std::nullopt;
            if (args->size() != 10) {
                *error = QStringLiteral("Diagnostic response for %1 has %2 bytes, expected 10")
                             .arg(address).arg(args->size());
                return std::nullopt;
            }
            queries.append(QJsonObject{{"pass", pass}, {"address", address},
                                       {"requestedPrefix", QString::fromLatin1(prefix.toHex())},
                                       {"returnedArguments", QString::fromLatin1(args->toHex())},
                                       {"echoMatches", args->left(3) == prefix},
                                       {"functionLengthValid", byte(*args, 4) <= 5}});
        }
    }
    return QJsonObject{{"diagnosticOnly", true}, {"usableAsBackup", false}, {"writesPerformed", false},
                       {"device", QJsonObject{{"vendor", "1532"}, {"product", "00b4"},
                                             {"firmware", QString::fromLatin1(firmware->toHex())}}},
                       {"queries", queries}};
}

std::optional<QJsonObject> backup(const Read &read, QString *error) {
    std::optional<QJsonObject> first;
    QJsonArray evidence;
    for (int pass = 0; pass != 2; ++pass) {
        const auto serial = read(0x0082, {}, 16, error);
        if (!serial) return std::nullopt;
        if (serial->size() != 16 || serial->trimmed().isEmpty() || *serial == QByteArray(16, 0)) {
            *error = QStringLiteral("No stable receiver serial was returned; cannot make a device-bound backup");
            return std::nullopt;
        }
        const auto firmware = read(0x0081, {}, 4, error);
        if (!firmware) return std::nullopt;
        if (firmware->size() != 4) {
            *error = QStringLiteral("Malformed firmware readback");
            return std::nullopt;
        }
        for (quint16 command : {quint16(0x058a), quint16(0x0580)}) {
            const auto profiles = read(command, {}, 1, error);
            if (!profiles) return std::nullopt;
            if (*profiles != QByteArray(1, 1)) {
                *error = QStringLiteral("Only a verified single-profile Naga layout is supported");
                return std::nullopt;
            }
        }
        QJsonObject assignments;
        QJsonObject replies;
        for (const auto &address : assignmentAddresses()) {
            const auto prefix = buttonPrefix(address);
            const auto args = read(GetButton, prefix, 10, error);
            if (!args) return std::nullopt;
            const bool normal = address.endsWith("/0");
            if (args->size() != 10 || (normal && (args->left(3) != prefix || byte(*args, 4) > 5))) {
                *error = QStringLiteral("Malformed or mismatched button-function readback for %1: "
                                        "requested prefix=%2, returned arguments=%3 (%4 bytes); backup refused")
                             .arg(address, QString::fromLatin1(prefix.toHex()), QString::fromLatin1(args->toHex()))
                             .arg(args->size());
                return std::nullopt;
            }
            replies.insert(address, QString::fromLatin1(args->toHex()));
            if (normal)
                assignments.insert(address, QString::fromLatin1(args->mid(3).toHex()));
        }
        QJsonObject current{{"schemaVersion", 2}, {"profile", 1}, {"scope", "normal-layer-only"},
                            {"hypershiftVerified", false},
                            {"device", QJsonObject{{"vendor", "1532"}, {"product", "00b4"},
                                 {"serialSha256", QString::fromLatin1(QCryptographicHash::hash(*serial, QCryptographicHash::Sha256).toHex())},
                                 {"firmware", QString::fromLatin1(firmware->toHex())}}},
                            {"assignments", assignments}, {"readEvidence", replies}};
        if (first && *first != current) {
            *error = QStringLiteral("Onboard settings changed between independent reads; backup refused");
            return std::nullopt;
        }
        evidence.append(replies);
        first = current;
    }
    first->insert("capturedAt", QDateTime::currentDateTimeUtc().toString(Qt::ISODateWithMs));
    first->insert("verifiedReads", 2);
    first->insert("readEvidence", evidence);
    first->insert("limitation", "Only normal-layer functions are verified. Hypershift query replies are retained as opaque evidence; "
                               "independent Hypershift state and effective behavior are not established.");
    return first;
}

std::optional<QJsonObject> dpiPlan(const QJsonObject &saved, const QJsonObject &current, QString *error) {
    if (!validBackup(saved, error) || !validBackup(current, error))
        return std::nullopt;
    if (saved.value("device") != current.value("device") || saved.value("assignments") != current.value("assignments") ||
        saved.value("readEvidence") != current.value("readEvidence")) {
        *error = QStringLiteral("Backup does not match the connected receiver's current assignments; no settings changed");
        return std::nullopt;
    }
    const auto assignments = saved.value("assignments").toObject();
    for (auto it = assignments.begin(); it != assignments.end(); ++it) {
        if (it.key() == "0b/0" || it.key() == "0c/0")
            continue;
        const auto fn = QByteArray::fromHex(it.value().toString().toLatin1());
        if ((byte(fn, 0) == 2 || byte(fn, 0) == 0x0d) && byte(fn, 1) >= 2 &&
            (byte(fn, 3) == 0x72 || byte(fn, 3) == 0x73)) {
            *error = QStringLiteral("F23/F24 are already used by %1; no DPI plan created").arg(it.key());
            return std::nullopt;
        }
    }
    QJsonArray changes;
    for (int button : {0x0b, 0x0c}) {
        const QString address = QStringLiteral("%1/0").arg(button, 2, 16, QLatin1Char('0'));
        const auto desired = QByteArray::fromHex(button == 0x0b ? "02020072000000" : "02020073000000");
        changes.append(QJsonObject{{"button", button}, {"layer", 0},
                                   {"slot", button == 0x0b ? "btn-dpi-up" : "btn-dpi-down"},
                                   {"key", button == 0x0b ? "F23" : "F24"},
                                   {"originalFunction", assignments.value(address)},
                                   {"proposedFunction", QString::fromLatin1(desired.toHex())}});
    }
    return QJsonObject{{"writesPerformed", false}, {"changes", changes}, {"profile", 1},
                      {"scope", "normal-layer-only"}, {"hypershiftVerified", false},
                      {"restoreCommand", "0x020c with each original 7-byte function; no reset or firmware flash"},
                      {"notice", "Plan only. Normal-layer DPI behavior would become F23/F24. Only two normal-layer addresses would be written; "
                                 "all other normal-layer assignments must remain unchanged. Independent Hypershift storage/behavior cannot "
                                 "be verified, so unchanged effective Hypershift behavior is NOT promised. "
                                 "A verified backup and specific user approval are required before executing writes. "
                                 "Emitted keys alone do not implement held workspace layers."}};
}

std::optional<QJsonObject> changeDpi(const QJsonObject &saved, const QJsonObject &before, DpiOperation operation,
                                   const WriteDpi &write, const Snapshot &snapshot, QString *error) {
    if (!validBackup(saved, error) || !validBackup(before, error))
        return std::nullopt;
    const auto original = saved.value("assignments").toObject();
    for (int button : {0x0b, 0x0c}) {
        const auto address = QStringLiteral("%1/0").arg(button, 2, 16, QLatin1Char('0'));
        if (original.value(address).toString().toLatin1() != originalDpi(button).toHex()) {
            *error = QStringLiteral("This guarded operation requires backed-up DPI next/previous functions");
            return std::nullopt;
        }
    }
    const auto eligible = [&](const QJsonObject &state, QString *reason) {
        if (!validBackup(state, reason)) return false;
        if (state.value("device") != saved.value("device")) {
            *reason = QStringLiteral("Connected receiver identity/firmware does not match the verified backup");
            return false;
        }
        const auto values = state.value("assignments").toObject();
        for (auto it = original.begin(); it != original.end(); ++it) {
            if (it.key() == "0b/0" || it.key() == "0c/0") {
                const int button = it.key() == "0b/0" ? 0x0b : 0x0c;
                if (values.value(it.key()) == it.value() ||
                    values.value(it.key()).toString().toLatin1() == mappedDpi(button).toHex())
                    continue;
            } else if (values.value(it.key()) == it.value()) {
                continue;
            }
            *reason = QStringLiteral("Unexpected current normal assignment at %1; refusing to overwrite it").arg(it.key());
            return false;
        }
        return true;
    };
    if (operation == DpiOperation::Apply) {
        if (!dpiPlan(saved, before, error)) return std::nullopt;
    } else if (!eligible(before, error)) {
        return std::nullopt;
    }
    auto expected = before.value("assignments").toObject();
    QJsonArray written;
    const auto failAndRestore = [&](const QString &failure) -> std::optional<QJsonObject> {
        QString reason;
        auto state = snapshot(&reason);
        if (!state || !eligible(*state, &reason)) {
            *error = failure + "; automatic restoration refused/unverified: " + reason +
                     ". Preserve the original backup; do not reset the mouse.";
            return std::nullopt;
        }
        QString acknowledgements;
        for (int button : {0x0b, 0x0c}) {
            const auto address = QStringLiteral("%1/0").arg(button, 2, 16, QLatin1Char('0'));
            if (state->value("assignments").toObject().value(address) == original.value(address)) continue;
            if (!write(button, originalDpi(button), &reason))
                acknowledgements += QStringLiteral(" Restore acknowledgement %1: %2.").arg(button).arg(reason);
        }
        state = snapshot(&reason);
        if (state && eligible(*state, &reason) && state->value("assignments").toObject() == original)
            *error = failure + "; original normal-layer assignments restored and independently verified." + acknowledgements;
        else
            *error = failure + "; original normal-layer restoration NOT verified: " + reason + acknowledgements +
                     ". Preserve the original backup; do not reset the mouse.";
        return std::nullopt;
    };
    for (int button : {0x0b, 0x0c}) {
        const auto address = QStringLiteral("%1/0").arg(button, 2, 16, QLatin1Char('0'));
        const auto desired = operation == DpiOperation::Apply ? mappedDpi(button) : originalDpi(button);
        if (expected.value(address).toString().toLatin1() == desired.toHex()) continue;
        QString reason;
        if (!write(button, desired, &reason))
            return failAndRestore(QStringLiteral("DPI write %1 failed: %2").arg(button).arg(reason));
        written.append(button);
        expected.insert(address, QString::fromLatin1(desired.toHex()));
        const auto observed = snapshot(&reason);
        if (!observed || !eligible(*observed, &reason))
            return failAndRestore(QStringLiteral("DPI write %1 readback/preservation check failed: %2").arg(button).arg(reason));
        if (observed->value("assignments").toObject() != expected)
            return failAndRestore(QStringLiteral("DPI write %1 did not produce the expected complete normal-layer state").arg(button));
    }
    return QJsonObject{{"operation", operation == DpiOperation::Apply ? "apply-dpi" : "restore-dpi"},
                       {"writesPerformed", !written.isEmpty()}, {"writtenButtons", written},
                       {"normalLayerVerified", true}, {"hypershiftVerified", false}, {"scope", "normal-layer-only"},
                       {"notice", "Other normal assignments verified unchanged. Independent Hypershift behavior remains unverified. "
                                  "F23/F24 are emitted keys, not held workspace layers."}};
}

int run(const QStringList &arguments) {
    const bool makeBackup = arguments == QStringList{"onboard-backup", "--json"};
    const bool makeProbe = arguments == QStringList{"onboard-probe", "--json"};
    const bool makePlan = arguments.size() == 4 && arguments[0] == "onboard-plan" && arguments[1] == "--backup" &&
                          arguments[3] == "--json";
    const bool apply = arguments.size() == 7 && arguments[0] == "onboard-apply-dpi" &&
                       arguments[5] == "--confirm-dpi-to-f23-f24";
    const bool restore = arguments.size() == 7 && arguments[0] == "onboard-restore-dpi" &&
                         arguments[5] == "--confirm-restore-dpi";
    const bool mutate = (apply || restore) && arguments[1] == "--backup" && arguments[3] == "--sha256" &&
                        hex(arguments[4], 32) && arguments[6] == "--json";
    auto fail = [](const QString &error) { QTextStream(stderr) << error << '\n'; return 1; };
    if (!makeBackup && !makePlan && !makeProbe && !mutate)
        return fail(QStringLiteral("Usage: mouse-remapperd onboard-backup --json | onboard-probe --json | onboard-plan --backup FILE --json\n"
                                   "       onboard-apply-dpi --backup FILE --sha256 HASH --confirm-dpi-to-f23-f24 --json\n"
                                   "       onboard-restore-dpi --backup FILE --sha256 HASH --confirm-restore-dpi --json\n"
                                   "Backup/probe/plan are read-only. Only explicit confirmed DPI commands can write; no firmware updates or resets."));
    QJsonObject saved;
    if (makePlan || mutate) {
        QString error;
        const auto bytes = readFile(arguments[2], &error);
        if (!bytes) return fail(error);
        if (mutate && QString::fromLatin1(QCryptographicHash::hash(*bytes, QCryptographicHash::Sha256).toHex()) !=
                          arguments[4].toLower())
            return fail(QStringLiteral("Backup SHA-256 does not match the explicitly approved file; no vendor query sent"));
        QJsonParseError parseError;
        const auto document = QJsonDocument::fromJson(*bytes, &parseError);
        if (!document.isObject() || parseError.error != QJsonParseError::NoError)
            return fail(QStringLiteral("Backup is not a JSON object"));
        saved = document.object();
        if (!validBackup(saved, &error))
            return fail(error);
    }
    Channel channel;
    QString error;
    if (!channel.open(&error))
        return fail(error);
    const Read read = [&](quint16 command, const QByteArray &args, int size, QString *reason) {
        return channel.read(command, args, size, reason);
    };
    const auto current = makeProbe ? probe(read, &error) : backup(read, &error);
    if (!current)
        return fail(error);
    const auto result = mutate ? changeDpi(saved, *current, apply ? DpiOperation::Apply : DpiOperation::Restore,
                                         [&](int button, const QByteArray &function, QString *reason) {
                                             return channel.writeDpi(button, function, reason);
                                         },
                                         [&](QString *reason) { return backup(read, reason); }, &error)
                              : makePlan ? dpiPlan(saved, *current, &error) : current;
    if (!result)
        return fail(error);
    QTextStream(stdout) << QJsonDocument(*result).toJson(QJsonDocument::Indented);
    return 0;
}
} // namespace cs::mouse::onboard
