// SPDX-License-Identifier: GPL-2.0-or-later
// Human-readable names for bindings, for the cheatsheet overlay and editors:
// a binding's own "label" when it has one, otherwise one made from what it
// does (Kdenlive catalog titles, readable chords like "Ctrl+Z", media names).
#pragma once

#include "config.h"

#include <QString>
#include <QVariantMap>
#include <functional>

namespace cs {

struct LabelEnv {
    std::function<QString(const QString &mode)> modeValue;  // current value of a daemon mode
    QVariantMap context;                                     // Kdenlive's context (may be empty)
};

QString prettyKeyName(int code);         // "Volume up", "Page Up", "F14", "A"
QString prettyChord(const KeyChord &c);  // "Ctrl+Shift+F14"
QString prettyIdentifier(const QString &id);  // "razor_tool" -> "Razor tool", "liftAxis" -> "Lift axis"

// The binding's "label", or the automatic one.
QString bindingLabel(const Binding &b, const LabelEnv &env);
QString autoLabel(const Binding &b, const LabelEnv &env);
// Extra state worth showing next to the label: a cycle's current value. Empty if none.
QString bindingState(const Binding &b, const LabelEnv &env);
// Short kind name: keys, mouse, action, control, request, cycle, command, cheatsheet, none.
QString bindingKindName(Binding::Kind k);

} // namespace cs
