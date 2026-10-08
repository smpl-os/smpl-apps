// SPDX-License-Identifier: GPL-2.0-or-later
#include "keysink.h"

#include <QHash>

#include <cstdio>
#include <linux/input-event-codes.h>

namespace cs {

QStringList mouseActionNames()
{
    return {QStringLiteral("left"),     QStringLiteral("right"),      QStringLiteral("middle"),
            QStringLiteral("back"),     QStringLiteral("forward"),    QStringLiteral("wheel-up"),
            QStringLiteral("wheel-down"), QStringLiteral("wheel-left"), QStringLiteral("wheel-right")};
}

std::optional<QList<InputTriple>> mouseEvents(const QString &action)
{
    static const QHash<QString, int> buttons{{QStringLiteral("left"), BTN_LEFT},   {QStringLiteral("right"), BTN_RIGHT},
                                             {QStringLiteral("middle"), BTN_MIDDLE}, {QStringLiteral("back"), BTN_SIDE},
                                             {QStringLiteral("forward"), BTN_EXTRA}};
    const InputTriple syn{EV_SYN, SYN_REPORT, 0};
    if (buttons.contains(action)) {
        const int b = buttons.value(action);
        return QList<InputTriple>{{EV_KEY, b, 1}, syn, {EV_KEY, b, 0}, syn};
    }
    // Wheel up and right are positive, as on a physical mouse.
    if (action == QLatin1String("wheel-up")) {
        return QList<InputTriple>{{EV_REL, REL_WHEEL, 1}, syn};
    }
    if (action == QLatin1String("wheel-down")) {
        return QList<InputTriple>{{EV_REL, REL_WHEEL, -1}, syn};
    }
    if (action == QLatin1String("wheel-right")) {
        return QList<InputTriple>{{EV_REL, REL_HWHEEL, 1}, syn};
    }
    if (action == QLatin1String("wheel-left")) {
        return QList<InputTriple>{{EV_REL, REL_HWHEEL, -1}, syn};
    }
    return std::nullopt;
}

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

void RecordingKeySink::mouse(const QString &action)
{
    taps << QStringLiteral("mouse:") + action;
    if (m_print) {
        std::printf("  -> mouse %s\n", qPrintable(action));
        std::fflush(stdout);
    }
}

} // namespace cs
