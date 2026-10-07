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
};

struct BoardProfile {
    QString id;
    QString name;
    QString source;  // measured | template | config | firmware
    int rows = 0, columns = 0;
    QList<BoardKey> keys;
    QList<BoardKnob> knobs;
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
