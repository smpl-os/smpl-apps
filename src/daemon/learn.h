// SPDX-License-Identifier: GPL-2.0-or-later
// Interactive pad verification: prompts for every physical control, records
// the chord it emits, compares with the flashed scheme and writes the map.
#pragma once

#include "boardprofile.h"

#include "decoder.h"
#include "hardwaremap.h"

#include <QElapsedTimer>
#include <QHash>
#include <QObject>

class QSocketNotifier;

namespace cs {

class PadDevice;

struct LearnTarget {
    QString control;
    Role role = Role::Key;
    QString prompt;
    QString name() const { return PadTarget{control, role}.name(); }
};
QList<LearnTarget> learnTargets(const BoardProfile &layout);

struct LearnReport {
    HardwareMap map;
    QStringList table;        // human-readable lines
    QString numbering;        // keys-then-knobs | vendor-twelve | custom
    int captured = 0;
    QStringList duplicates;   // chords seen for more than one target
    QStringList foreign;      // chords outside the flashed 24-chord scheme
    bool complete() const { return captured == 24 && duplicates.isEmpty(); }
};
LearnReport evaluateLearn(const QList<LearnTarget> &targets, const QHash<int, KeyChord> &captured);
bool writeHardwareMap(const LearnReport &r, const QString &path, QString *error);

class PadVerifier : public QObject
{
    Q_OBJECT
public:
    PadVerifier(PadDevice *device, const QString &outPath, bool write, const BoardProfile &layout, QObject *parent = nullptr);
    void start();

Q_SIGNALS:
    void finished(int exitCode);

private:
    void onChord(const ChordEvent &e);
    void onStdin();
    void prompt();
    void finish();

    PadDevice *m_device;
    QString m_outPath;
    bool m_write;
    QList<LearnTarget> m_targets;
    int m_index = 0;
    QHash<int, KeyChord> m_captured;
    QHash<quint32, qint64> m_downAt;
    QList<qint64> m_holdUsec;
    QElapsedTimer m_sincePrompt;
    QSocketNotifier *m_stdin = nullptr;
    bool m_waitingForDevice = true;
};

} // namespace cs
