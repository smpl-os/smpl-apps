// SPDX-License-Identifier: GPL-2.0-or-later
#include "featurelist.h"
#include "boardprofile.h"
#include "config.h"
#include "kdenlivecontract.h"
#include "keynames.h"
#include "keysink.h"
#include "settingsservice.h"

#include <QCoreApplication>
#include <QJsonArray>

namespace cs {

QJsonObject featuresJson()
{
    auto kind = [](const char *name, const char *example, const char *description, bool knobOk = true) {
        return QJsonObject{{QStringLiteral("kind"), QLatin1String(name)},
                           {QStringLiteral("example"), QLatin1String(example)},
                           {QStringLiteral("description"), QLatin1String(description)},
                           {QStringLiteral("perDetent"), knobOk}};
    };
    const QJsonArray kinds{
        kind("keys", R"("ctrl+z" | {"keys": ["ctrl+k", "x"]})", "Tap a chord or a sequence through the virtual keyboard"),
        kind("mouse", R"({"mouse": "wheel-down"})", "A mouse button click or one wheel detent through the virtual pointer"),
        kind("action", R"({"action": "mark_in", "fallback": "i"})", "A Kdenlive action by id (kdenlive profiles); fallback keys only with keyFallback"),
        kind("control", R"({"control": "playhead.jog", "scale": 1, "accel": 3})", "A continuous Kdenlive control, coalesced (kdenlive profiles)"),
        kind("request", R"({"request": "colorwheel.reset", "params": {"wheel": "lift"}})", "Invoke a Kdenlive command (kdenlive profiles)"),
        kind("cycle", R"({"cycle": "liftAxis"})", "Advance a mode defined under \"modes\""),
        kind("command", R"({"command": ["notify-send", "hello"]})", "Start a program (argv, no shell)", false),
        kind("cheatsheet", R"({"cheatsheet": "toggle"} | {"cheatsheet": "hold"})", "Show what each input does now as an overlay: toggle, or while held (keys and knob presses only)", false),
        kind("none", R"("none")", "Explicitly unbound; stops the fall-through to the global profile"),
    };
    QJsonArray boards;
    for (const BoardProfile &p : builtinBoardProfiles()) {
        boards.append(QJsonObject{{QStringLiteral("id"), p.id}, {QStringLiteral("name"), p.name}, {QStringLiteral("keys"), int(p.keys.size())},
                                  {QStringLiteral("knobs"), int(p.knobs.size())}, {QStringLiteral("source"), p.source}});
    }
    return QJsonObject{
        {QStringLiteral("daemonVersion"), QCoreApplication::applicationVersion()},
        {QStringLiteral("apiVersion"), int(SettingsService::kApiVersion)},
        {QStringLiteral("dbus"), QJsonObject{{QStringLiteral("service"), QLatin1String(SettingsService::kService)},
                                             {QStringLiteral("path"), QLatin1String(SettingsService::kPath)},
                                             {QStringLiteral("interface"), QLatin1String(SettingsService::kInterface)}}},
        {QStringLiteral("bindingKinds"), kinds},
        {QStringLiteral("keyNames"), QJsonArray::fromStringList(keyNames())},
        {QStringLiteral("modifierNames"), QJsonArray::fromStringList(modifierNames())},
        {QStringLiteral("chordSyntax"), QStringLiteral("modifier+modifier+KEY, case-insensitive, e.g. ctrl+shift+F14; a sequence is a list")},
        {QStringLiteral("mouseNames"), QJsonArray::fromStringList(mouseActionNames())},
        {QStringLiteral("slots"), QJsonObject{{QStringLiteral("maxKeys"), 16},
                                              {QStringLiteral("maxKnobs"), 3},
                                              {QStringLiteral("keys"), QStringLiteral("key1..key16 (row-major)")},
                                              {QStringLiteral("knobEvents"), QJsonArray{QStringLiteral("turn"), QStringLiteral("ccw"), QStringLiteral("cw"), QStringLiteral("press"),
                                                                                        QStringLiteral("shift.turn"), QStringLiteral("shift.ccw"), QStringLiteral("shift.cw")}},
                                              {QStringLiteral("knob"), QStringLiteral("knob1..knob3 with .turn/.ccw/.cw/.press or .shift.turn/.shift.ccw/.shift.cw (turning while held)")}}},
        {QStringLiteral("inputEvents"), QJsonArray{QStringLiteral("press"), QStringLiteral("release"), QStringLiteral("ccw"), QStringLiteral("cw")}},
        {QStringLiteral("layouts"), QJsonObject{{QStringLiteral("builtin"), boards},
                                                {QStringLiteral("custom"), QStringLiteral(R"({"keys": 0..16, "knobs": 0..3, "columns": 1..8})")},
                                                // The config's "layout" overrides the firmware's board.
                                                {QStringLiteral("precedence"), QJsonArray{QStringLiteral("config"), QStringLiteral("firmware"),
                                                                                          QStringLiteral("hardware-map"), QStringLiteral("default")}}}},
        {QStringLiteral("device"), QJsonObject{{QStringLiteral("usb"), QStringLiteral("1189:8890")},
                                               {QStringLiteral("serial"), QStringLiteral("empty: the first pad found")},
                                               {QStringLiteral("input"), QJsonArray{QStringLiteral("auto"), QStringLiteral("evdev"), QStringLiteral("raw")}}}},
        {QStringLiteral("plugins"), QJsonArray{QStringLiteral("keys"), QStringLiteral("command"), QStringLiteral("kdenlive")}},
        {QStringLiteral("cheatsheet"), QJsonObject{{QStringLiteral("modes"), QJsonArray{QStringLiteral("toggle"), QStringLiteral("hold")}},
                                                   {QStringLiteral("slots"), QStringLiteral("keyN or knobN.press")},
                                                   // What an unset option means, from CheatsheetOptions itself.
                                                   {QStringLiteral("defaults"), [] {
                                                        const CheatsheetOptions d;
                                                        return QJsonObject{{QStringLiteral("opacity"), d.opacity},
                                                                           {QStringLiteral("autoHideMs"), d.effectiveAutoHideMs()},
                                                                           {QStringLiteral("position"), d.position}};
                                                    }()},
                                                   {QStringLiteral("options"), QJsonObject{{QStringLiteral("opacity"), QStringLiteral("0.05..1, default %1").arg(CheatsheetOptions().opacity)},
                                                                                           {QStringLiteral("autoHideMs"), QStringLiteral("0..600000; unset = 8000, 0 = until hidden; restarted by pad input; not while a hold key holds it")},
                                                                                           {QStringLiteral("position"), QJsonArray::fromStringList(CheatsheetOptions::positions())},
                                                                                           {QStringLiteral("eww"), QStringLiteral(R"(true | false | {"enabled", "variable": "pad_sheet", "window", "binary": "eww", "config"}; over run --eww / --eww-window NAME / --eww-config DIR)")}}},
                                                   {QStringLiteral("eww"), QJsonObject{{QStringLiteral("update"), QStringLiteral("eww [--config DIR] update VARIABLE=<GetCheatsheet JSON>: on show, change and hide, and hidden at start and exit")},
                                                                                       {QStringLiteral("open"), QStringLiteral("eww [--config DIR] open WINDOW --anchor A after a successful update on show; reopened when the position changes")},
                                                                                       {QStringLiteral("close"), QStringLiteral("eww [--config DIR] close WINDOW before the update on hide")},
                                                                                       {QStringLiteral("anchors"), [] {
                                                                                            QJsonObject a;
                                                                                            for (const QString &pos : CheatsheetOptions::positions()) {
                                                                                                a.insert(pos, EwwHook::anchorFor(pos));
                                                                                            }
                                                                                            return a;
                                                                                        }()}}},
                                                   {QStringLiteral("label"), QStringLiteral("any binding object may carry \"label\"; otherwise one is made from what it does")}}},
        {QStringLiteral("kdenlive"), QJsonObject{{QStringLiteral("interface"), cs::contract::kInterface},
                                                 {QStringLiteral("catalog"), QStringLiteral("control-surfaced list-actions --json")}}},
    };
}

} // namespace cs
