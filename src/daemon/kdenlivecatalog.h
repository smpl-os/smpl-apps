// SPDX-License-Identifier: GPL-2.0-or-later
// What a mapping editor can offer for Kdenlive without a running Kdenlive:
// the K23-MR1a curated action candidates (k23-contract-mr1a-actions.md, 71
// ids) and the contract's controls and commands. A running Kdenlive's
// ListActions stays authoritative (it returns only registered ids).
#pragma once

#include <QJsonObject>
#include <QList>
#include <QString>
#include <QStringList>

namespace cs::catalog {

struct Action {
    QString id;
    QString text;      // approximate English label; only the id is normative
    QString shortcut;  // Kdenlive's usual default, for keyFallback suggestions
    bool checkable = false;
    QString group;     // playback | navigation | monitor | zone | markers | editing | tools | selection | keyframes | history
    bool editing = false;   // one of the 30 editing-writer actions (clarification E)
    bool playback = false;  // disabled during the trimming preview (clarification F)
};

struct Control {
    QString name;
    QString stage;    // MR1 | MR2 | MR3
    QString unit;     // what one detent means
    bool editing = false;  // needs target + gesture (undoable)
    QString description;
};

struct Command {
    QString name;
    QString stage;
    QString description;
};

const QList<Action> &actions();
const QList<Control> &controls();
const QList<Command> &commands();
QStringList editingActionIds();
QStringList playbackActionIds();
QJsonObject toJson();  // {contract, actions[], controls[], commands[]}

} // namespace cs::catalog
