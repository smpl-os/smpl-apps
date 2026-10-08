// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QJsonObject>
#include <QList>
#include <QString>
#include <optional>

namespace cs {

// Physical layout of a pad: where each key and knob sits, and which firmware
// slot each input uses (keys row-major, then knob n ccw/press/cw).
struct BoardKey {
    QString control;  // key1..
    int slot = 0;
    int row = 0, column = 0;
};

struct BoardKnob {
    QString control;  // knob1..
    int ccw = 0, press = 0, cw = 0;
    int row = 0, column = 0;
    // Pressing this knob holds one of its encoder lines low (measured), so a
    // turn while it is pressed carries no direction on any firmware.
    bool pressPinsEncoder = false;
};

struct BoardProfile {
    QString id;
    QString name;
    QString source;  // measured | template | config | firmware
    int rows = 0, columns = 0;
    QList<BoardKey> keys;
    QList<BoardKnob> knobs;
    // Whether turning a knob while it is pressed reaches the host. The
    // control-surface firmware (2.0.x) ignores those turns: "shift" bindings
    // never fire on this board.
    bool turnsWhilePressed = true;
    // Controls the pad reports one at a time (sy181: the TM1650 matrix, keys
    // 2-15 and the knob presses): pressing one releases the one held before,
    // and releasing it reports the earlier one down again.
    QStringList oneAtATime;
    int slotCount() const { return int(keys.size() + 3 * knobs.size()); }
    QJsonObject toJson() const;
};

// keys in rows of `columns`, knobs stacked in one extra column on the right.
BoardProfile gridProfile(const QString &id, const QString &name, int keys, int knobs, int columns);
QList<BoardProfile> builtinBoardProfiles();
// Layout from the controls a config or hardware map names (key1.., knob1..):
// the measured profile when the counts match it, otherwise a grid.
BoardProfile profileForControls(const QStringList &controls, const QString &source);
std::optional<BoardProfile> builtinBoardProfile(const QString &id);

} // namespace cs
