// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QByteArray>
#include <QJsonObject>
#include <QJsonValue>
#include <QList>
#include <QString>
#include <QStringList>
#include <optional>

namespace cs {

// Simple options a person (or the Settings app) can change without editing
// the config: `control-surfaced set KEY VALUE`, D-Bus SetOption.
struct OptionSpec {
    enum Type { Enum, String, Number, Integer, Bool };
    QString key;              // "input", "cheatsheet.opacity" ...
    QStringList path;         // where it lives in the config: {"device", "input"}
    Type type = String;
    QStringList values;       // Enum: the allowed values
    double min = 0, max = 0;  // Number, Integer
    QString help;
    QStringList labels;       // Enum: what a settings UI shows for each value (optional)
    QJsonObject toJson() const;
};
const QList<OptionSpec> &settableOptions();
std::optional<OptionSpec> optionSpec(const QString &key);
// The value as typed (raw, 0.35, true, "") for an option; aliases accepted
// (input: keymap = evdev).
std::optional<QJsonValue> parseOptionValue(const OptionSpec &spec, const QString &text, QString *error);

// JSON with comments, edited in place: the value at path is replaced, or the
// member (and any missing parent objects) inserted after the object's last
// member. Comments, layout and trailing commas elsewhere are kept as they are.
std::optional<QByteArray> setJsoncValue(const QByteArray &text, const QStringList &path, const QJsonValue &value, QString *error);
// The value at path (Undefined when absent or the text does not parse).
QJsonValue jsoncValue(const QByteArray &text, const QStringList &path);

// set KEY VALUE on a config file: validated as a whole config, written
// atomically with a backup of the previous version (ConfigStore). A missing
// file starts from the built-in default. Shared by the CLI and SetOption.
struct OptionChange {
    bool ok = false;
    QString code;      // unknown-option | invalid-value | invalid-config | hash-mismatch | io
    QString message;
    QString key;
    QJsonValue oldValue, newValue;
    bool changed = false;
    QString path, hash, backup;
    QByteArray text;   // the config as written
    QJsonObject toJson() const;
};
OptionChange setOption(const QString &configPath, const QString &key, const QString &valueText);
// The option's value in the file (or the built-in default when there is none),
// and what it means when absent ("auto" for input ...).
QJsonObject getOption(const QString &configPath, const QString &key);

} // namespace cs
