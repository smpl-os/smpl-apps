// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "config.h"
#include "engine.h"
#include "keysink.h"
#include "windowtracker.h"
#include <QMap>
#include <QSet>
#include <functional>
#include <linux/input-event-codes.h>
#include <tuple>

namespace cs::mouse {

constexpr quint16 Vendor = 0x1532;
constexpr quint16 Product = 0x00b4;

struct SourceKey {
    QString source;
    int code = 0;
    int type = EV_KEY;
    int direction = 0;
    bool operator<(const SourceKey &other) const {
        return std::tie(source, type, code, direction) < std::tie(other.source, other.type, other.code, other.direction);
    }
    bool operator==(const SourceKey &other) const { return !(*this < other) && !(other < *this); }
    static SourceKey relative(const QString &source, int code, int value);
};

class SetupFlow {
public:
    static QStringList steps();
    void start();
    void cancel();
    bool active() const { return m_index >= 0; }
    bool review() const { return active() && m_index == steps().size(); }
    bool armed() const { return m_armed; }
    bool captured() const { return !target().isEmpty() && m_inputs.contains(target()); }
    int step() const { return m_index + 1; }
    QString target() const { return active() && !review() ? steps().at(m_index) : QString(); }
    bool arm(QString *error);
    bool capture(const SourceKey &input, QString *error);
    bool next(bool skip, QString *error);
    void retry();
    const QMap<QString, SourceKey> &inputs() const { return m_inputs; }

private:
    int m_index = -1;
    bool m_armed = false;
    QMap<QString, SourceKey> m_inputs;
};

class GridCalibration {
public:
    void start() {
        m_inputs.clear();
        m_active = true;
    }
    void cancel() {
        m_inputs.clear();
        m_active = false;
    }
    bool active() const { return m_active; }
    bool complete() const { return m_active && m_inputs.size() == 12; }
    QString target() const { return m_active && !complete() ? QStringLiteral("grid-%1").arg(m_inputs.size() + 1) : QString(); }
    bool capture(const SourceKey &key, QString *error);
    const QMap<QString, SourceKey> &inputs() const { return m_inputs; }

private:
    QMap<QString, SourceKey> m_inputs;
    bool m_active = false;
};

struct Mapping {
    QList<Profile> profiles;
    QMap<SourceKey, QString> learned;
    static std::optional<Mapping> parse(const QByteArray &text, QString *error);
    std::optional<Binding> binding(const QString &slot, const WindowInfo &window, bool focusAvailable) const;
    QString slot(const SourceKey &key, bool pointer) const;
    QString relativeSlot(const QString &source, int code, int value) const;
    static QString wheelSlot(int axis, int direction);
    QStringList unlearned() const;
    Config config() const {
        Config result;
        result.profiles = profiles;
        return result;
    }
};

// Pure event router: hardware I/O and process launches are injected by the
// host.
class Router : public KeySink {
public:
    explicit Router(KdenliveClient *client = nullptr);
    Mapping mapping;
    Engine &engine() { return m_engine; }
    void setMapping(const Mapping &value);
    void setWindow(const WindowInfo &window, bool focusAvailable = true);
    bool identify = false;
    std::function<void(bool pointer, const InputTriple &)> output;
    std::function<void(const Binding &)> command;
    std::function<void(const QString &, const SourceKey &)> pressed;
    std::function<void(const SourceKey &)> released;
    std::function<void(const QString &, const QString &, const InputTriple &)> scrolled;

    // True means consumed; everything else is forwarded unchanged by the host.
    bool key(const SourceKey &key, bool pointer, int value);
    bool consume(const QString &source, bool pointer, const InputTriple &event);
    void releaseAll();
    void disconnected();
    bool hasOtherKeyDown(const SourceKey &key) const;
    void tap(const KeyChord &chord) override;
    void mouse(const QString &action) override;

private:
    struct Held {
        bool consumed = false;
        QList<QPair<bool, int>> codes;
    };
    QMap<SourceKey, Held> m_held;
    QMap<QPair<bool, int>, int> m_refs;
    struct WheelFrame {
        int legacy = 0;
        int highResolution = 0;
        bool hasHighResolution = false;
    };
    QMap<QPair<QString, int>, WheelFrame> m_wheels;
    QMap<QPair<QString, int>, int> m_wheelRemainder;
    QMap<QString, QSet<QPair<int, int>>> m_identifiedWheels;
    Engine m_engine;
    bool m_focusAvailable = false;
    void applyMapping();
    void send(bool pointer, int type, int code, int value);
    void acquire(Held &held, bool pointer, int code);
    void release(Held &held);
    bool wheel(const QString &source, const InputTriple &event);
    void flushWheels(const QString &source);
    void dispatchPulse(const QString &slot);
};

} // namespace cs::mouse
