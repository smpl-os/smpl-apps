// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "keynames.h"

#include <QList>
#include <QObject>
#include <QStringList>
#include <optional>

namespace cs {

// Mouse actions a binding can name: {"mouse": "left"}.
QStringList mouseActionNames();  // left right middle back forward wheel-up wheel-down wheel-left wheel-right

// The input events (type, code, value) one mouse action sends on a pointer
// device, SYN_REPORTs included: a click is press, sync, release, sync; a wheel
// action is one detent.
struct InputTriple {
    int type = 0, code = 0, value = 0;
    bool operator==(const InputTriple &) const = default;
};
std::optional<QList<InputTriple>> mouseEvents(const QString &action);

class KeySink : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;
    // Press modifiers, press+release key, release modifiers, as one gesture.
    virtual void tap(const KeyChord &chord) = 0;
    // One mouse action from mouseActionNames() (click or one wheel detent).
    virtual void mouse(const QString &action) { Q_UNUSED(action) }
    virtual bool isReady() const { return true; }
};

// Records taps (tests) and optionally prints them (dry-run / simulate).
class RecordingKeySink : public KeySink
{
    Q_OBJECT
public:
    explicit RecordingKeySink(bool print = false, QObject *parent = nullptr);
    void tap(const KeyChord &chord) override;
    void mouse(const QString &action) override;  // recorded as "mouse:<action>"
    QStringList taps;

private:
    bool m_print;
};

} // namespace cs
