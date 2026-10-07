// SPDX-License-Identifier: GPL-2.0-or-later
#include "learn.h"
#include "paddevice.h"

#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QSocketNotifier>
#include <algorithm>
#include <cstdio>
#include <unistd.h>

namespace cs {

QList<LearnTarget> learnTargets()
{
    QList<LearnTarget> t;
    for (int i = 1; i <= 15; ++i) {
        const int row = (i - 1) / 5 + 1;
        const int col = (i - 1) % 5 + 1;
        t << LearnTarget{QStringLiteral("key%1").arg(i), Role::Key, QStringLiteral("Press the key in row %1, column %2").arg(row).arg(col)};
    }
    for (int k = 1; k <= 3; ++k) {
        const QString knob = QStringLiteral("knob%1").arg(k);
        t << LearnTarget{knob, Role::Ccw, QStringLiteral("Turn knob %1 ONE click counter-clockwise").arg(k)};
        t << LearnTarget{knob, Role::Cw, QStringLiteral("Turn knob %1 ONE click clockwise").arg(k)};
        t << LearnTarget{knob, Role::Press, QStringLiteral("Press knob %1 down (click it)").arg(k)};
    }
    return t;
}

LearnReport evaluateLearn(const QList<LearnTarget> &targets, const QHash<int, KeyChord> &captured)
{
    LearnReport r;
    const HardwareMap a = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
    const HardwareMap b = HardwareMap::fromScheme(ch552::Numbering::VendorTwelve);
    bool matchA = true;
    bool matchB = true;
    QHash<quint32, QString> seen;
    for (int i = 0; i < targets.size(); ++i) {
        const LearnTarget &t = targets.at(i);
        if (!captured.contains(i)) {
            r.table << QStringLiteral("%1  (skipped)").arg(t.name(), -12);
            matchA = matchB = false;
            continue;
        }
        const KeyChord c = captured.value(i);
        ++r.captured;
        if (seen.contains(c.id())) {
            r.duplicates << QStringLiteral("%1 also emitted by %2").arg(chordName(c), seen.value(c.id()));
        }
        seen.insert(c.id(), t.name());
        const auto ea = a.lookup(c);
        const auto eb = b.lookup(c);
        if (!ea) {
            r.foreign << QStringLiteral("%1 -> %2").arg(t.name(), chordName(c));
        }
        const PadTarget want{t.control, t.role};
        matchA = matchA && ea && *ea == want;
        matchB = matchB && eb && *eb == want;
        r.table << QStringLiteral("%1 %2 scheme slot as %3")
                       .arg(t.name(), -12)
                       .arg(chordName(c), -16)
                       .arg(ea ? ea->name() : QStringLiteral("(not in scheme)"));
        r.map.insert(c, want);
    }
    r.numbering = matchA ? QStringLiteral("keys-then-knobs") : matchB ? QStringLiteral("vendor-twelve") : QStringLiteral("custom");
    return r;
}

bool writeHardwareMap(const LearnReport &r, const QString &path, QString *error)
{
    QDir().mkpath(QFileInfo(path).absolutePath());
    if (QFile::exists(path)) {
        const QString backup = path + QStringLiteral(".bak-") + QDateTime::currentDateTime().toString(QStringLiteral("yyyyMMdd-HHmmss"));
        if (!QFile::copy(path, backup)) {
            if (error) {
                *error = QStringLiteral("cannot back up %1").arg(path);
            }
            return false;
        }
    }
    QJsonObject o = r.map.toJson();
    o.insert(QStringLiteral("numbering"), r.numbering);
    o.insert(QStringLiteral("learned"), QDateTime::currentDateTime().toString(Qt::ISODate));
    QSaveFile f(path);
    if (!f.open(QIODevice::WriteOnly)) {
        if (error) {
            *error = QStringLiteral("cannot write %1").arg(path);
        }
        return false;
    }
    f.write(QJsonDocument(o).toJson());
    return f.commit();
}

PadVerifier::PadVerifier(PadDevice *device, const QString &outPath, bool write, QObject *parent)
    : QObject(parent)
    , m_device(device)
    , m_outPath(outPath)
    , m_write(write)
    , m_targets(learnTargets())
{
    connect(m_device, &PadDevice::chordEvent, this, &PadVerifier::onChord);
    connect(m_device, &PadDevice::connected, this, [this] {
        if (m_waitingForDevice) {
            m_waitingForDevice = false;
            std::printf("\nPad grabbed: its keys reach only this tool until it exits.\n"
                        "Hold the pad so the 15 keys form 3 rows of 5; knob 1 is the first knob (left-most / top-most).\n"
                        "Type s+Enter to skip a step, b+Enter to redo the previous one, q+Enter to stop.\n\n");
            prompt();
        }
    });
    connect(m_device, &PadDevice::disconnected, this, [] { std::printf("\n(pad disconnected - plug it back in)\n"); });
}

void PadVerifier::start()
{
    if (::isatty(0)) {
        m_stdin = new QSocketNotifier(0, QSocketNotifier::Read, this);
        connect(m_stdin, &QSocketNotifier::activated, this, &PadVerifier::onStdin);
    }
    std::printf("Waiting for the 1189:8890 pad...\n");
    std::fflush(stdout);
    m_device->start();
}

void PadVerifier::prompt()
{
    if (m_index >= m_targets.size()) {
        finish();
        return;
    }
    std::printf("[%2d/24] %s ... ", m_index + 1, qPrintable(m_targets.at(m_index).prompt));
    std::fflush(stdout);
    m_sincePrompt.start();
}

void PadVerifier::onChord(const ChordEvent &e)
{
    if (!e.down) {
        const auto it = m_downAt.constFind(e.chord.id());
        if (it != m_downAt.constEnd()) {
            m_holdUsec << (e.usec - *it);
            m_downAt.remove(e.chord.id());
        }
        return;
    }
    m_downAt.insert(e.chord.id(), e.usec);
    // Knob clicks can bounce into a second detent; ignore events right after a capture.
    if (m_waitingForDevice || m_index >= m_targets.size() || m_sincePrompt.elapsed() < 350) {
        return;
    }
    m_captured.insert(m_index, e.chord);
    std::printf("%s\n", qPrintable(chordName(e.chord)));
    ++m_index;
    prompt();
}

void PadVerifier::onStdin()
{
    char buf[64];
    const ssize_t n = ::read(0, buf, sizeof buf);
    if (n <= 0) {
        m_stdin->setEnabled(false);
        return;
    }
    const char c = buf[0];
    if (c == 'q') {
        std::printf("\n");
        m_index = int(m_targets.size());
        finish();
    } else if (c == 's' && m_index < m_targets.size()) {
        std::printf("skipped\n");
        ++m_index;
        prompt();
    } else if (c == 'b' && m_index > 0) {
        --m_index;
        m_captured.remove(m_index);
        std::printf("\n");
        prompt();
    }
}

void PadVerifier::finish()
{
    const LearnReport r = evaluateLearn(m_targets, m_captured);
    std::printf("\n==== result ====\n");
    for (const auto &l : r.table) {
        std::printf("  %s\n", qPrintable(l));
    }
    std::printf("\ncaptured %d/24, slot numbering: %s\n", r.captured, qPrintable(r.numbering));
    for (const auto &d : r.duplicates) {
        std::printf("  DUPLICATE: %s\n", qPrintable(d));
    }
    for (const auto &f : r.foreign) {
        std::printf("  NOT FROM THE FLASHED SCHEME: %s\n", qPrintable(f));
    }
    if (!m_holdUsec.isEmpty()) {
        auto h = m_holdUsec;
        std::sort(h.begin(), h.end());
        std::printf("median key-down time: %.1f ms (the pad replays taps; holding a key does %s)\n", h.at(h.size() / 2) / 1000.0,
                    h.at(h.size() / 2) > 150000 ? "seem to be tracked" : "not extend it");
    }
    int code = r.complete() ? 0 : 1;
    if (m_write && r.captured > 0 && r.duplicates.isEmpty()) {
        QString err;
        if (writeHardwareMap(r, m_outPath, &err)) {
            std::printf("wrote %s\n", qPrintable(m_outPath));
        } else {
            std::printf("ERROR: %s\n", qPrintable(err));
            code = 2;
        }
    } else if (m_write) {
        std::printf("hardware map NOT written (nothing captured or duplicates present)\n");
    }
    std::fflush(stdout);
    Q_EMIT finished(code);
}

} // namespace cs
