// SPDX-License-Identifier: GPL-2.0-or-later
#include "service.h"
#include <QDBusConnection>
#include <QDBusError>
#include <QDBusMessage>
#include <QDateTime>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QProcess>
#include <QSaveFile>
#include <QUuid>
#include <linux/input-event-codes.h>

namespace cs::mouse {
namespace {
QString json(const QJsonObject &object) { return QString::fromUtf8(QJsonDocument(object).toJson(QJsonDocument::Compact)); }
} // namespace

Service::Service(QString path)
    : m_path(std::move(path)), m_kdenlive(QDBusConnection::sessionBus()), m_router(&m_kdenlive), m_devices(m_router),
      m_watcher(QString(), QDBusConnection::sessionBus(), QDBusServiceWatcher::WatchForUnregistration) {
    connect(&m_router.engine(), &Engine::message, this, [this](const QString &text) {
        m_notice = text;
        qInfo().noquote() << text;
    });
    connect(&m_kdenlive, &KdenliveClient::message, this, [this](const QString &text) {
        m_notice = text;
        qInfo().noquote() << text;
    });
    connect(&m_watcher, &QDBusServiceWatcher::serviceUnregistered, this, [this](const QString &owner) {
        if (owner == m_owner)
            clearModes();
    });
    m_devices.failed = [this](const QString &reason) {
        m_error = reason;
        m_failed = true;
        qWarning().noquote() << reason;
        QTimer::singleShot(0, this, [this] {
            m_devices.close();
            clearModes();
        });
    };
    m_router.command = [this](const Binding &binding) {
        if (binding.argv.isEmpty())
            return;
        QStringList argv = binding.argv;
        for (auto &argument : argv)
            argument = expandHome(argument);
        QProcess process;
        process.setProgram(argv.takeFirst());
        process.setArguments(argv);
        process.setStandardInputFile(QProcess::nullDevice());
        process.setStandardOutputFile(QProcess::nullDevice());
        process.setStandardErrorFile(QProcess::nullDevice());
        if (!process.startDetached()) {
            m_notice = QStringLiteral("Mouse command failed to launch: ") + process.errorString();
            qWarning().noquote() << m_notice;
        }
    };
    m_router.pressed = [this](const QString &slot, const SourceKey &key) { pressed(slot, key); };
    m_router.released = [this](const SourceKey &key) { released(key); };
    m_router.scrolled = [this](const QString &slot, const QString &source, const InputTriple &event) { scrolled(slot, source, event); };
    connect(&m_timer, &QTimer::timeout, this, &Service::tick);
    m_timer.setInterval(1000);
}

void Service::start() {
    if (!qEnvironmentVariableIsEmpty("HYPRLAND_INSTANCE_SIGNATURE")) {
        m_tracker = std::make_unique<HyprlandTracker>();
        connect(m_tracker.get(), &WindowTracker::activeWindowChanged, this, [this](const WindowInfo &window) {
            m_router.setWindow(window);
            m_focusAvailable = true;
            m_focusError.clear();
        });
        connect(m_tracker.get(), &WindowTracker::message, this, [this](const QString &message) {
            if (message.contains("disconnected")) {
                m_router.setWindow({}, false);
                m_focusAvailable = false;
                m_focusError = QStringLiteral("Hyprland focus disconnected; using Global assignments only");
            }
        });
        m_focusError = QStringLiteral("Waiting for Hyprland focus; using Global assignments only");
        m_tracker->start();
    } else {
        m_focusError = QStringLiteral("Per-app profiles require Hyprland; using Global assignments only");
    }
    tick();
    m_timer.start();
}

void Service::stop() {
    m_timer.stop();
    clearModes();
    m_devices.close();
}

void Service::load() {
    QFile file(m_path);
    QByteArray bytes = "{}";
    if (file.exists()) {
        if (!file.open(QIODevice::ReadOnly)) {
            m_error = QStringLiteral("Read mouse.json: ") + file.errorString();
            m_configValid = false;
            return;
        }
        bytes = file.readAll();
        if (file.error() != QFileDevice::NoError) {
            m_error = QStringLiteral("Read mouse.json: ") + file.errorString();
            m_configValid = false;
            return;
        }
    }
    if (m_configValid && bytes == m_text)
        return;
    QString reason;
    auto mapping = Mapping::parse(bytes, &reason);
    m_router.releaseAll();
    m_text = bytes;
    m_configValid = bool(mapping);
    if (mapping) {
        m_router.setMapping(*mapping);
        m_error.clear();
    } else {
        m_error = reason;
    }
}

void Service::tick() {
    if (!m_owner.isEmpty() && m_lease.elapsed() > 10000)
        clearModes();
    if (m_setup.armed() && m_learnTime.elapsed() > 15000) {
        m_setup.retry();
        m_notice = QStringLiteral("No matching input captured. Click Capture next press to retry, or Skip to keep this control's "
                                  "existing mapping. Some current onboard assignments emit no evdev input; no firmware was changed.");
        updateMode();
    } else if (!m_learning.isEmpty() && m_learnTime.elapsed() > 15000) {
        m_notice = QStringLiteral("No safe input captured for %1; no calibration saved. "
                                  "Onboard-only DPI/scroll-mode controls need a firmware key assignment before evdev can identify them. "
                                  "No onboard settings were changed.")
                       .arg(m_learning);
        m_learning.clear();
        m_calibration.cancel();
        updateMode();
    }
    load();
    if (!m_configValid) {
        clearModes();
        m_devices.close();
        return;
    }
    if (m_failed) {
        m_failed = false;
        return; // Give the compositor a chance to process the removed virtual
                // devices.
    }
    if (!m_devices.connected()) {
        QString reason;
        if (!m_devices.attach(&reason)) {
            if (m_error != reason)
                qWarning().noquote() << reason;
            m_error = reason;
        } else {
            m_error.clear();
        }
    }
}

QString Service::Status() {
    if (calledFromDBus() && message().service() == m_owner)
        m_lease.restart();
    QJsonArray learned;
    for (auto it = m_router.mapping.learned.begin(); it != m_router.mapping.learned.end(); ++it)
        learned.append(it.value());
    QJsonArray unlearned;
    for (const auto &slot : m_router.mapping.unlearned())
        unlearned.append(slot);
    return json({{"apiVersion", 1},
                 {"connected", m_devices.present()},
                 {"ready", m_configValid && m_devices.connected() && !m_failed},
                 {"error", m_error},
                 {"notice", m_notice},
                 {"configPath", m_path},
                 {"learned", learned},
                 {"unlearned", unlearned},
                 {"identify", m_identify},
                 {"learning", m_learning},
                 {"lastSlot", m_lastSlot},
                 {"lastInput", m_lastInput},
                 {"sequence", qint64(m_sequence)},
                 {"events", m_events},
                 {"calibrating", m_calibration.active()},
                 {"guidedSetup", true},
                 {"setup", setupStatus()},
                 {"focusBackend", m_focusAvailable ? "hyprland" : "global-only"},
                 {"kdenliveApi", true},
                 {"kdenliveAvailable", m_router.engine().kdenliveActive()},
                 {"kdenliveProfile", m_router.engine().activeProfile() && m_router.engine().activeProfile()->kdenlive},
                 {"focusWarning", m_focusError}});
}

QJsonObject Service::setupStatus() const {
    QJsonArray rows;
    for (const auto &slot : SetupFlow::steps()) {
        const auto found = m_setup.inputs().constFind(slot);
        rows.append(found == m_setup.inputs().constEnd()
                        ? QStringLiteral("%1: skipped / existing mapping retained").arg(slot)
                        : QStringLiteral("%1: %2, %3 code=%4%5")
                              .arg(slot, found->source, found->type == EV_KEY ? QStringLiteral("EV_KEY") : QStringLiteral("EV_REL"))
                              .arg(found->code)
                              .arg(found->type == EV_REL ? QStringLiteral(" direction=%1").arg(found->direction) : QString()));
    }
    return {{"active", m_setup.active()},
            {"phase", !m_setup.active() ? "" : m_setup.review() ? "review" : m_setup.armed() ? "armed" : m_setup.captured() ? "captured" : "idle"},
            {"target", m_setup.target()},
            {"step", qMin(m_setup.step(), int(SetupFlow::steps().size()))},
            {"total", int(SetupFlow::steps().size())},
            {"captured", int(m_setup.inputs().size())},
            {"rows", rows}};
}

bool Service::claim() {
    const QString caller = calledFromDBus() ? message().service() : QStringLiteral("test");
    if (!m_owner.isEmpty() && caller != m_owner) {
        sendErrorReply(QDBusError::AccessDenied, QStringLiteral("Another Settings client owns mouse identify/learning"));
        return false;
    }
    if (!m_devices.connected() || !m_configValid || m_failed) {
        sendErrorReply(QDBusError::Failed, QStringLiteral("Mouse remapper is not ready: ") + m_error);
        return false;
    }
    if (m_owner.isEmpty()) {
        m_owner = caller;
        if (calledFromDBus())
            m_watcher.addWatchedService(caller);
    }
    m_lease.restart();
    return true;
}

void Service::Learn(const QString &slot) {
    static const QRegularExpression allowed(
        QStringLiteral("^(grid-([1-9]|1[0-2])|btn-(dpi|dpi-up|dpi-down|scroll-mode|back|forward)|wheel-tilt-(left|right))$"));
    if (!allowed.match(slot).hasMatch()) {
        sendErrorReply(QDBusError::InvalidArgs, QStringLiteral("Select a grid, DPI or tilt button to learn; "
                                                               "primary buttons have fixed codes"));
        return;
    }
    if (!claim())
        return;
    m_setup.cancel();
    m_calibration.cancel();
    m_learning = slot;
    m_learnTime.restart();
    m_notice = QStringLiteral("Press the selected physical button once; primary clicks remain usable");
    updateMode();
}

void Service::Calibrate() {
    if (!claim())
        return;
    m_setup.cancel();
    m_identify = true;
    m_calibration.start();
    m_learning = m_calibration.target();
    m_learnTime.restart();
    m_notice = QStringLiteral("Side-grid calibration: press physical button 1, then 2 through 12 as highlighted. "
                              "Codes are captured, never guessed; all 12 are saved together.");
    updateMode();
}

void Service::StartSetup() {
    if (!claim())
        return;
    m_calibration.cancel();
    m_learning.clear();
    m_identify = true;
    m_setup.start();
    m_setupText = m_text;
    m_notice = QStringLiteral("Click Capture next press, move the pointer over the diagram, then press and release the highlighted "
                              "physical control. Navigation clicks are ignored until capture is armed. Nothing is saved yet.");
    updateMode();
}

void Service::ArmSetup() {
    if (!claim())
        return;
    QString error;
    if (m_router.hasOtherKeyDown({}) || !m_setup.arm(&error)) {
        rejectSetup(error.isEmpty() ? QStringLiteral("Release all mouse buttons before arming capture") : error);
        return;
    }
    m_learnTime.restart();
    m_notice = QStringLiteral("Capture armed: move the pointer over the diagram, then press and release %1. "
                              "Primary clicks, motion and Escape on your keyboard remain usable.")
                   .arg(m_setup.target());
    updateMode();
}

void Service::rejectSetup(const QString &error) {
    m_notice = error;
    qWarning().noquote() << error;
    sendErrorReply(QDBusError::Failed, error);
}

void Service::advanceSetup(bool skip) {
    if (!claim())
        return;
    QString error;
    if (!m_setup.next(skip, &error)) {
        rejectSetup(error);
        return;
    }
    m_notice = m_setup.review()
                   ? QStringLiteral("Review captured codes below. Save setup applies only captured positions; skipped positions keep "
                                    "their existing mappings. Cancel discards this setup.")
                   : QStringLiteral("Click Capture next press for %1, or Skip if its current onboard assignment emits no usable input. "
                                    "Nothing has been saved.")
                         .arg(m_setup.target());
    updateMode();
}

void Service::NextSetup() { advanceSetup(false); }
void Service::SkipSetup() { advanceSetup(true); }

void Service::RetrySetup() {
    if (!claim())
        return;
    if (!m_setup.active() || m_setup.review()) {
        rejectSetup(QStringLiteral("Setup is not capturing a control"));
        return;
    }
    m_setup.retry();
    m_notice = QStringLiteral("Capture cleared for this step. Click Capture next press to try again; saved mappings are unchanged.");
    updateMode();
}

void Service::SaveSetup() {
    if (!claim())
        return;
    if (!m_setup.review()) {
        rejectSetup(QStringLiteral("Finish the setup steps and review captured codes before saving"));
        return;
    }
    QString error;
    if (!saveLearnedButtons(m_path, m_setup.inputs(), &error, &m_setupText)) {
        rejectSetup(error);
        return;
    }
    load();
    m_setup.cancel();
    m_notice = QStringLiteral("Mouse setup saved with a verified backup. Identify now selects captured positions; skipped controls "
                              "retain their previous behavior.");
    updateMode();
}

void Service::CancelSetup() {
    if (!m_owner.isEmpty() && !claim())
        return;
    clearModes();
    m_notice = QStringLiteral("Mouse setup cancelled. No captured mappings were saved.");
}

void Service::CancelLearn() {
    if (!m_owner.isEmpty() && !claim())
        return;
    m_learning.clear();
    m_setup.cancel();
    m_calibration.cancel();
    updateMode();
}

void Service::SetIdentify(bool enabled) {
    if (!claim())
        return;
    m_identify = enabled;
    m_learning.clear();
    m_setup.cancel();
    m_calibration.cancel();
    m_lastSlot.clear();
    m_lastInput.clear();
    ++m_sequence;
    updateMode();
    if (!enabled)
        clearModes();
}

void Service::updateMode() {
    m_capture.reset();
    m_captureChord = false;
    m_router.releaseAll();
    m_router.identify = m_identify || !m_learning.isEmpty();
    if (!m_router.identify && !m_owner.isEmpty()) {
        m_watcher.removeWatchedService(m_owner);
        m_owner.clear();
    }
}

void Service::clearModes() {
    m_identify = false;
    m_learning.clear();
    m_calibration.cancel();
    m_setup.cancel();
    if (!m_owner.isEmpty())
        m_watcher.removeWatchedService(m_owner);
    m_owner.clear();
    updateMode();
}

bool saveLearnedButton(const QString &path, const QString &slot, const SourceKey &key, QString *out) {
    return saveLearnedButtons(path, {{slot, key}}, out);
}

bool saveLearnedButtons(const QString &path, const QMap<QString, SourceKey> &inputs, QString *out, const QByteArray *expected) {
    if (inputs.isEmpty()) {
        *out = QStringLiteral("No inputs captured; nothing saved");
        return false;
    }
    const QFileInfo info(path);
    const QString target = info.isSymbolicLink() ? info.canonicalFilePath() : path;
    if (target.isEmpty()) {
        *out = QStringLiteral("Cannot resolve mouse configuration symlink");
        return false;
    }
    QFile original(target);
    const bool exists = original.exists();
    QByteArray before = "{}";
    if (exists) {
        if (!original.open(QIODevice::ReadOnly)) {
            *out = QStringLiteral("Read learning configuration: ") + original.errorString();
            return false;
        }
        before = original.readAll();
        if (original.error() != QFileDevice::NoError) {
            *out = original.errorString();
            return false;
        }
    }
    if (expected && before != *expected) {
        *out = QStringLiteral("mouse.json changed during setup. Cancel and restart setup to preserve the other edit; nothing saved.");
        return false;
    }
    if (!Mapping::parse(before, out))
        return false;
    auto root = QJsonDocument::fromJson(stripJsonComments(before)).object();
    auto devices = root.value("devices").toObject();
    auto device = devices.value("mmo-12").toObject();
    auto learned = device.value("learned").toObject();
    for (auto it = inputs.begin(); it != inputs.end(); ++it)
        learned.remove(it.key());
    for (auto input = inputs.begin(); input != inputs.end(); ++input) {
        const auto &slot = input.key();
        const auto &key = input.value();
        for (auto it = learned.begin(); it != learned.end(); ++it) {
            const auto value = it.value().toObject();
            const SourceKey previous{value.value("source").toString(), value.value("code").toInt(),
                                     value.contains("type") ? value.value("type").toInt() : EV_KEY,
                                     value.value("direction").toInt()};
            if (it.key() != slot && previous == key) {
                *out = QStringLiteral("This input already belongs to %1; no mapping changed").arg(it.key());
                return false;
            }
        }
        QJsonObject value{{"source", key.source}, {"code", key.code}};
        if (key.type != EV_KEY) {
            value.insert("type", key.type);
            value.insert("direction", key.direction);
        }
        learned.insert(slot, value);
    }
    device.insert("learned", learned);
    devices.insert("mmo-12", device);
    root.insert("devices", devices);
    const QByteArray bytes = QJsonDocument(root).toJson(QJsonDocument::Indented);
    if (!Mapping::parse(bytes, out))
        return false;
    if (!QDir().mkpath(QFileInfo(target).absolutePath())) {
        *out = QStringLiteral("Cannot create mouse configuration directory");
        return false;
    }
    // Keep the exact previous document, including comments, before learning.
    if (exists) {
        const QString dir = QFileInfo(target).absolutePath() + "/backups";
        if (!QDir().mkpath(dir)) {
            *out = QStringLiteral("Cannot create mouse learning backup directory");
            return false;
        }
        const QString backupPath = dir + "/mouse-learn-" + QDateTime::currentDateTimeUtc().toString("yyyyMMdd-HHmmss") + "-" +
                                   QUuid::createUuid().toString(QUuid::WithoutBraces) + ".json";
        QSaveFile backup(backupPath);
        if (!backup.open(QIODevice::WriteOnly) || !backup.setPermissions(original.permissions()) || backup.write(before) != before.size() ||
            !backup.commit()) {
            *out = QStringLiteral("Back up mouse configuration: ") + backup.errorString();
            return false;
        }
        QFile verified(backupPath);
        if (!verified.open(QIODevice::ReadOnly) || verified.readAll() != before || verified.error() != QFileDevice::NoError) {
            *out = QStringLiteral("Mouse configuration backup could not be verified; "
                                  "no learning saved");
            return false;
        }
    }
    QFile current(target);
    if (current.exists() != exists || (exists && (!current.open(QIODevice::ReadOnly) || current.readAll() != before))) {
        *out = QStringLiteral("mouse.json changed while learning; retry without "
                              "overwriting the other edit");
        return false;
    }
    QSaveFile saved(target);
    saved.setDirectWriteFallback(false);
    if (!saved.open(QIODevice::WriteOnly) || (exists && !saved.setPermissions(original.permissions())) ||
        saved.write(bytes) != bytes.size() || !saved.commit()) {
        *out = QStringLiteral("Save learned button: ") + saved.errorString();
        return false;
    }
    return true;
}

void Service::pressed(const QString &slot, const SourceKey &key) {
    if (!m_router.identify)
        return;
    if (m_setup.active()) {
        if (m_setup.armed()) {
            if (m_capture) {
                m_captureChord = true;
                m_notice = QStringLiteral("Multiple inputs detected. Release them and capture one physical control.");
            } else if (!modifierBit(key.code)) {
                m_capture = key;
                m_captureChord = m_router.hasOtherKeyDown(key);
            }
        }
    } else if (!m_learning.isEmpty()) {
        if (m_capture) {
            m_captureChord = true;
            m_notice = QStringLiteral("Multiple keys detected. Release them and press only the highlighted button; nothing saved.");
        } else if (key.code != BTN_LEFT && key.code != BTN_RIGHT && key.code != BTN_MIDDLE && !modifierBit(key.code)) {
            m_capture = key;
            m_captureChord = m_router.hasOtherKeyDown(key);
        }
    } else {
        m_notice = slot.isEmpty()
                       ? QStringLiteral("Unknown button: use Calibrate side grid, or select its position and Learn selected button")
                       : QStringLiteral("Identified %1").arg(slot);
    }
    reportInput(slot, key.source, EV_KEY, key.code, 1);
}

void Service::released(const SourceKey &key) {
    if (!m_capture || m_capture->source != key.source || m_capture->code != key.code)
        return;
    const bool chord = m_captureChord || m_router.hasOtherKeyDown(key);
    m_capture.reset();
    m_captureChord = false;
    if (m_setup.active()) {
        const QString target = m_setup.target();
        QString error;
        if (chord)
            error = QStringLiteral("Multiple keys were held. Release them and Try again; chord-emitting buttons cannot be captured.");
        else if (m_setup.capture(key, &error))
            m_notice = QStringLiteral("Captured %1 from %2: EV_KEY code=%3. Check the highlighted position, then Next or Try again. "
                                      "Nothing is saved until final review.")
                           .arg(target, key.source).arg(key.code);
        if (!error.isEmpty()) {
            m_notice = error;
            qWarning().noquote() << error;
        }
        updateMode();
        reportInput(error.isEmpty() ? target : QString(), key.source, EV_KEY, key.code, 0);
        return;
    }
    if (!m_learning.isEmpty() && m_router.identify) {
        QString reason;
        const QString target = m_learning;
        if (chord) {
            reason = QStringLiteral("Release other buttons and learn a single-key input; "
                                    "chord-emitting buttons are not supported");
        } else if (m_calibration.active()) {
            if (m_calibration.capture(key, &reason)) {
                m_lastSlot = target;
                if (m_calibration.complete()) {
                    if (saveLearnedButtons(m_path, m_calibration.inputs(), &reason)) {
                        load();
                        m_notice = QStringLiteral("All 12 side buttons calibrated. Identify now selects each position on the map.");
                    }
                    m_learning.clear();
                    m_calibration.cancel();
                } else {
                    m_learning = m_calibration.target();
                    m_notice = QStringLiteral(
                                   "Captured %1. Now press physical button %2 (highlighted). Nothing is saved until all 12 are captured.")
                                   .arg(target)
                                   .arg(m_calibration.inputs().size() + 1);
                    m_learnTime.restart();
                }
            }
        } else if (saveLearnedButton(m_path, target, key, &reason)) {
            load();
            m_lastSlot = target;
            m_notice = QStringLiteral("Learned %1. Assign its action below.").arg(target);
        }
        if (!reason.isEmpty()) {
            m_notice = reason;
            qWarning().noquote() << reason;
        }
        if (!m_calibration.active())
            m_learning.clear();
        updateMode();
    }
    reportInput(m_lastSlot, key.source, EV_KEY, key.code, 0);
}

void Service::scrolled(const QString &slot, const QString &source, const InputTriple &event) {
    if (!m_router.identify)
        return;
    if (m_setup.active()) {
        if (m_setup.armed()) {
            QString error;
            const QString target = m_setup.target();
            if (m_capture || m_router.hasOtherKeyDown({}))
                error = QStringLiteral("Release held buttons before capturing a wheel direction");
            else if (m_setup.capture(SourceKey::relative(source, event.code, event.value), &error))
                m_notice = QStringLiteral("Captured %1 from %2. Check the highlighted position, then Next or Try again. Nothing saved yet.")
                               .arg(target, source);
            if (!error.isEmpty()) {
                m_notice = error;
                qWarning().noquote() << error;
            }
        }
    } else if (!m_learning.isEmpty()) {
        m_notice = QStringLiteral("Wheel direction %1 detected. It already has a map position; "
                                  "press the highlighted physical button for %2, or Cancel learning.")
                       .arg(slot, m_learning);
    } else if (!slot.isEmpty()) {
        m_notice = QStringLiteral("Identified %1").arg(slot);
    } else {
        m_notice = QStringLiteral("Unknown relative input from %1: code %2, value %3. No physical position was guessed.")
                       .arg(source).arg(event.code).arg(event.value);
    }
    reportInput(slot, source, EV_REL, event.code, event.value);
}

void Service::reportInput(const QString &slot, const QString &source, int type, int code, int value) {
    m_lastSlot = slot;
    m_lastInput = QStringLiteral("%1: %2 code=%3 value=%4%5")
                      .arg(source, type == EV_REL ? QStringLiteral("EV_REL") : QStringLiteral("EV_KEY"))
                      .arg(code)
                      .arg(value)
                      .arg(slot.isEmpty() ? QString() : QStringLiteral(" (%1)").arg(slot));
    ++m_sequence;
    const QJsonObject event{{"slot", slot},   {"input", m_lastInput},          {"source", source}, {"type", type}, {"code", code},
                            {"value", value}, {"sequence", qint64(m_sequence)}};
    m_events.append(event);
    while (m_events.size() > 64)
        m_events.removeFirst();
    Q_EMIT InputEvent(json(event));
}

} // namespace cs::mouse
