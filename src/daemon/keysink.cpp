// SPDX-License-Identifier: GPL-2.0-or-later
#include "keysink.h"

#include <cstdio>

namespace cs {

RecordingKeySink::RecordingKeySink(bool print, QObject *parent)
    : KeySink(parent)
    , m_print(print)
{
}

void RecordingKeySink::tap(const KeyChord &chord)
{
    const QString n = chordName(chord);
    taps << n;
    if (m_print) {
        std::printf("  -> key %s\n", qPrintable(n));
        std::fflush(stdout);
    }
}

} // namespace cs
