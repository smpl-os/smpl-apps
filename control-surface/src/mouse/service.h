// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "device.h"
#include "kdenlivedbusclient.h"
#include <QDBusContext>
#include <QDBusServiceWatcher>
#include <QElapsedTimer>
#include <QJsonArray>
#include <QTimer>

namespace cs::mouse {

// Atomic learning write, independently testable without opening input devices.
bool saveLearnedButton(const QString &path, const QString &slot, const SourceKey &key, QString *error);
bool saveLearnedButtons(const QString &path, const QMap<QString, SourceKey> &inputs, QString *error,
                       const QByteArray *expected = nullptr);

class Service : public QObject, protected QDBusContext {
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.smplos.MouseRemapper1")
public:
    explicit Service(QString path);
    void start();
    void stop();

public Q_SLOTS:
    QString Status();
    void Learn(const QString &slot);
    void CancelLearn();
    void SetIdentify(bool enabled);
    void Calibrate();
    void StartSetup();
    void ArmSetup();
    void NextSetup();
    void SkipSetup();
    void RetrySetup();
    void SaveSetup();
    void CancelSetup();

Q_SIGNALS:
    void InputEvent(const QString &json);

private:
    QString m_path, m_error, m_focusError, m_learning, m_owner, m_lastSlot, m_lastInput, m_notice;
    QByteArray m_text;
    QByteArray m_setupText;
    bool m_configValid = false, m_identify = false, m_failed = false;
    quint64 m_sequence = 0;
    KdenliveDBusClient m_kdenlive;
    Router m_router;
    Devices m_devices;
    QDBusServiceWatcher m_watcher;
    QTimer m_timer;
    QElapsedTimer m_lease, m_learnTime;
    GridCalibration m_calibration;
    SetupFlow m_setup;
    std::optional<SourceKey> m_capture;
    bool m_captureChord = false;
    std::unique_ptr<WindowTracker> m_tracker;
    bool m_focusAvailable = false;
    void tick();
    void load();
    void pressed(const QString &slot, const SourceKey &key);
    void released(const SourceKey &key);
    void scrolled(const QString &slot, const QString &source, const InputTriple &event);
    void reportInput(const QString &slot, const QString &source, int type, int code, int value);
    QJsonArray m_events;
    bool claim();
    void clearModes();
    void updateMode();
    void advanceSetup(bool skip);
    void rejectSetup(const QString &error);
    QJsonObject setupStatus() const;
};

} // namespace cs::mouse
