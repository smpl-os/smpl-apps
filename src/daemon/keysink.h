// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "keynames.h"

#include <QObject>
#include <QStringList>

namespace cs {

class KeySink : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;
    // Press modifiers, press+release key, release modifiers, as one gesture.
    virtual void tap(const KeyChord &chord) = 0;
    virtual bool isReady() const { return true; }
};

// Records taps (tests) and optionally prints them (dry-run / simulate).
class RecordingKeySink : public KeySink
{
    Q_OBJECT
public:
    explicit RecordingKeySink(bool print = false, QObject *parent = nullptr);
    void tap(const KeyChord &chord) override;
    QStringList taps;

private:
    bool m_print;
};

} // namespace cs
