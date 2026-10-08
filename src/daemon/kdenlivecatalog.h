// SPDX-License-Identifier: GPL-2.0-or-later
// What a mapping editor can offer for Kdenlive without a running Kdenlive:
// the curated actions of K23 MR1a (71 ids) and MR1b-A (29 more, plus the
// camera, layout, tag and effect families), and the contract's controls and
// commands. A running Kdenlive's ListActions stays authoritative (it returns
// only registered ids, and "enabled" follows its UI state).
#pragma once

#include <QJsonArray>
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
    bool editing = false;   // an editing-writer action (MR1a clarification E, MR1b-A's additions and families)
    bool playback = false;  // disabled during the trimming preview (clarification F)
    QString family;         // camera | layout | tag (MR1b-A dynamic families); empty for fixed ids
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
QStringList editingActionIds();   // the fixed editing ids (39)
QStringList playbackActionIds();
// MR1b-A dynamic families: "camera" (activate_video_1..9), "layout"
// (load_layout1..9), "tag" (tag_<n>), "effect" (effect_<id>); empty otherwise.
QString actionFamily(const QString &id);
bool isEditingAction(const QString &id);  // fixed editing ids and the camera, tag, effect families
bool isOffered(const QString &id);        // a fixed id or a family member
QStringList excludedActionIds();          // named and refused by MR1b-A (dialogs, no undo, recording)
QJsonArray families();
QJsonObject toJson();  // {contract, actions[], controls[], commands[]}

} // namespace cs::catalog
