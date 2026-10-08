// SPDX-License-Identifier: GPL-2.0-or-later
#include "configedit.h"
#include "config.h"
#include "configstore.h"

#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <cmath>
#include <cstring>

namespace cs {

// ---------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------

QJsonObject OptionSpec::toJson() const
{
    static const char *types[] = {"enum", "string", "number", "integer", "bool"};
    QJsonObject o{{QStringLiteral("key"), key}, {QStringLiteral("path"), path.join(QLatin1Char('.'))},
                  {QStringLiteral("type"), QLatin1String(types[type])}, {QStringLiteral("help"), help}};
    if (type == Enum) {
        o.insert(QStringLiteral("values"), QJsonArray::fromStringList(values));
        if (labels.size() == values.size()) {
            QJsonObject l;
            for (int i = 0; i < values.size(); ++i) {
                l.insert(values[i], labels[i]);
            }
            o.insert(QStringLiteral("labels"), l);
        }
    }
    if (type == Number || type == Integer) {
        o.insert(QStringLiteral("min"), min);
        o.insert(QStringLiteral("max"), max);
    }
    return o;
}

const QList<OptionSpec> &settableOptions()
{
    static const QList<OptionSpec> list{
        {QStringLiteral("input"), {QStringLiteral("device"), QStringLiteral("input")}, OptionSpec::Enum,
         {QStringLiteral("auto"), QStringLiteral("evdev"), QStringLiteral("raw")}, 0, 0,
         QStringLiteral("how the daemon reads the pad: auto (raw on control-surface firmware 2.0.2+, evdev otherwise), evdev (the pad's keymap, compatible), raw (the firmware's own events, fastest; firmware 2.0.2+)"),
         {QStringLiteral("Automatic"), QStringLiteral("Keymap (compatible)"), QStringLiteral("Raw (fastest)")}},
        {QStringLiteral("serial"), {QStringLiteral("device"), QStringLiteral("serial")}, OptionSpec::String, {}, 0, 0,
         QStringLiteral("drive only the pad with this USB serial; \"\" = the first 1189:8890 pad found"), {}},
        {QStringLiteral("cheatsheet.opacity"), {QStringLiteral("cheatsheet"), QStringLiteral("opacity")}, OptionSpec::Number, {}, 0.05, 1,
         QStringLiteral("the overlay's opacity"), {}},
        {QStringLiteral("cheatsheet.autoHideMs"), {QStringLiteral("cheatsheet"), QStringLiteral("autoHideMs")}, OptionSpec::Integer, {}, 0, 600000,
         QStringLiteral("hide the overlay after this long without pad input; 0 = until hidden"), {}},
        {QStringLiteral("cheatsheet.position"), {QStringLiteral("cheatsheet"), QStringLiteral("position")}, OptionSpec::Enum,
         CheatsheetOptions::positions(), 0, 0, QStringLiteral("where the overlay sits"), {}},
        {QStringLiteral("cheatsheet.eww"), {QStringLiteral("cheatsheet"), QStringLiteral("eww")}, OptionSpec::Bool, {}, 0, 0,
         QStringLiteral("push the overlay into eww (over run --eww-window/--eww-config)"), {}},
        {QStringLiteral("settings.accelFactor"), {QStringLiteral("settings"), QStringLiteral("accelFactor")}, OptionSpec::Number, {}, 1, 10,
         QStringLiteral("how much further fast knob detents move (1 = never accelerate)"), {}},
        {QStringLiteral("settings.accelWindowMs"), {QStringLiteral("settings"), QStringLiteral("accelWindowMs")}, OptionSpec::Integer, {}, 5, 200,
         QStringLiteral("detents closer than this count as fast"), {}},
        {QStringLiteral("settings.keyRateHz"), {QStringLiteral("settings"), QStringLiteral("keyRateHz")}, OptionSpec::Integer, {}, 10, 1000,
         QStringLiteral("pace of key taps produced by knob detents"), {}},
    };
    return list;
}

std::optional<OptionSpec> optionSpec(const QString &key)
{
    for (const OptionSpec &s : settableOptions()) {
        if (s.key.compare(key, Qt::CaseInsensitive) == 0) {
            return s;
        }
    }
    return std::nullopt;
}

std::optional<QJsonValue> parseOptionValue(const OptionSpec &spec, const QString &text, QString *error)
{
    auto fail = [error](const QString &m) -> std::optional<QJsonValue> {
        if (error) {
            *error = m;
        }
        return std::nullopt;
    };
    const QString t = text.trimmed();
    switch (spec.type) {
    case OptionSpec::Enum: {
        QString v = t.toLower();
        if (spec.key == QLatin1String("input")) {
            if (v == QLatin1String("keymap")) {
                v = QStringLiteral("evdev");
            } else if (v == QLatin1String("automatic")) {
                v = QStringLiteral("auto");
            }
        }
        if (!spec.values.contains(v)) {
            return fail(QStringLiteral("%1 must be one of %2").arg(spec.key, spec.values.join(QStringLiteral(", "))));
        }
        return QJsonValue(v);
    }
    case OptionSpec::String:
        return QJsonValue(text);
    case OptionSpec::Bool:
        if (t == QLatin1String("true") || t == QLatin1String("on") || t == QLatin1String("yes") || t == QLatin1String("1")) {
            return QJsonValue(true);
        }
        if (t == QLatin1String("false") || t == QLatin1String("off") || t == QLatin1String("no") || t == QLatin1String("0")) {
            return QJsonValue(false);
        }
        return fail(QStringLiteral("%1 must be true or false").arg(spec.key));
    case OptionSpec::Number:
    case OptionSpec::Integer: {
        bool ok = false;
        const double d = t.toDouble(&ok);
        if (!ok || !std::isfinite(d) || d < spec.min || d > spec.max || (spec.type == OptionSpec::Integer && d != std::floor(d))) {
            return fail(QStringLiteral("%1 must be %2 from %3 to %4")
                            .arg(spec.key, spec.type == OptionSpec::Integer ? QStringLiteral("a whole number") : QStringLiteral("a number"))
                            .arg(spec.min)
                            .arg(spec.max));
        }
        return spec.type == OptionSpec::Integer ? QJsonValue(qint64(d)) : QJsonValue(d);
    }
    }
    return std::nullopt;
}

// ---------------------------------------------------------------------------------
// JSONC in place
// ---------------------------------------------------------------------------------

namespace {
struct Scanner {
    const QByteArray &t;
    qsizetype i = 0;

    bool at(char c) const { return i < t.size() && t.at(i) == c; }
    void ws()
    {
        for (;;) {
            while (i < t.size() && (t.at(i) == ' ' || t.at(i) == '\t' || t.at(i) == '\r' || t.at(i) == '\n')) {
                ++i;
            }
            if (i + 1 < t.size() && t.at(i) == '/' && t.at(i + 1) == '/') {
                while (i < t.size() && t.at(i) != '\n') {
                    ++i;
                }
                continue;
            }
            if (i + 1 < t.size() && t.at(i) == '/' && t.at(i + 1) == '*') {
                i += 2;
                while (i + 1 < t.size() && !(t.at(i) == '*' && t.at(i + 1) == '/')) {
                    ++i;
                }
                i = qMin(i + 2, t.size());
                continue;
            }
            return;
        }
    }
    bool string(QString *out)
    {
        if (!at('"')) {
            return false;
        }
        const qsizetype start = i++;
        while (i < t.size() && t.at(i) != '"') {
            i += t.at(i) == '\\' ? 2 : 1;
        }
        if (i >= t.size()) {
            return false;
        }
        ++i;
        if (out) {
            *out = QJsonDocument::fromJson("[" + t.mid(start, i - start) + "]").array().at(0).toString();
        }
        return true;
    }
    bool value()
    {
        ws();
        if (i >= t.size()) {
            return false;
        }
        const char c = t.at(i);
        if (c == '"') {
            return string(nullptr);
        }
        if (c == '{' || c == '[') {
            const char close = c == '{' ? '}' : ']';
            ++i;
            for (;;) {
                ws();
                if (at(close)) {
                    ++i;
                    return true;
                }
                if (c == '{') {
                    if (!string(nullptr)) {
                        return false;
                    }
                    ws();
                    if (!at(':')) {
                        return false;
                    }
                    ++i;
                }
                if (!value()) {
                    return false;
                }
                ws();
                if (at(',')) {
                    ++i;
                    continue;
                }
                if (at(close)) {
                    ++i;
                    return true;
                }
                return false;
            }
        }
        const qsizetype start = i;
        while (i < t.size() && !std::strchr(",}] \t\r\n/", t.at(i))) {
            ++i;
        }
        return i > start;
    }
};

struct Member {
    QString key;
    qsizetype keyStart = 0, valueStart = 0, valueEnd = 0;
};
struct ObjectInfo {
    qsizetype open = 0, close = 0;
    QList<Member> members;
};

std::optional<ObjectInfo> objectAt(Scanner &s)
{
    s.ws();
    if (!s.at('{')) {
        return std::nullopt;
    }
    ObjectInfo o;
    o.open = s.i++;
    for (;;) {
        s.ws();
        if (s.at('}')) {
            o.close = s.i++;
            return o;
        }
        Member m;
        m.keyStart = s.i;
        if (!s.string(&m.key)) {
            return std::nullopt;
        }
        s.ws();
        if (!s.at(':')) {
            return std::nullopt;
        }
        ++s.i;
        s.ws();
        m.valueStart = s.i;
        if (!s.value()) {
            return std::nullopt;
        }
        m.valueEnd = s.i;
        o.members << m;
        s.ws();
        if (s.at(',')) {
            ++s.i;
            continue;
        }
        if (s.at('}')) {
            o.close = s.i++;
            return o;
        }
        return std::nullopt;
    }
}

QByteArray serialized(const QJsonValue &v)
{
    const QByteArray a = QJsonDocument(QJsonArray{v}).toJson(QJsonDocument::Compact);
    return a.mid(1, a.size() - 2);
}

// Objects on one line, spaced like the example config: { "input": "raw" }.
QByteArray inlineJson(const QJsonValue &v)
{
    if (!v.isObject()) {
        return serialized(v);
    }
    const QJsonObject o = v.toObject();
    QByteArrayList parts;
    for (auto it = o.begin(); it != o.end(); ++it) {
        parts << serialized(QJsonValue(it.key())) + ": " + inlineJson(it.value());
    }
    return parts.isEmpty() ? QByteArray("{}") : "{ " + parts.join(", ") + " }";
}

// Whitespace from the start of the line to pos, if only whitespace precedes it.
std::optional<QByteArray> lineIndent(const QByteArray &t, qsizetype pos)
{
    qsizetype b = pos;
    while (b > 0 && t.at(b - 1) != '\n') {
        --b;
    }
    const QByteArray lead = t.mid(b, pos - b);
    for (char c : lead) {
        if (c != ' ' && c != '\t') {
            return std::nullopt;
        }
    }
    return lead;
}
} // namespace

std::optional<QByteArray> setJsoncValue(const QByteArray &text, const QStringList &path, const QJsonValue &value, QString *error)
{
    auto fail = [error](const QString &m) -> std::optional<QByteArray> {
        if (error) {
            *error = m;
        }
        return std::nullopt;
    };
    if (path.isEmpty()) {
        return fail(QStringLiteral("empty path"));
    }
    Scanner s{text};
    if (text.startsWith("\xEF\xBB\xBF")) {
        s.i = 3;  // a UTF-8 BOM, as the config parser accepts
    }
    const QByteArray nl = text.contains("\r\n") ? QByteArrayLiteral("\r\n") : QByteArrayLiteral("\n");
    for (int depth = 0; depth < path.size(); ++depth) {
        const auto obj = objectAt(s);
        if (!obj) {
            return fail(depth == 0 ? QStringLiteral("the config is not a JSON object")
                                   : QStringLiteral("%1 is not an object").arg(path.mid(0, depth).join(QLatin1Char('.'))));
        }
        const QString &key = path.at(depth);
        const Member *found = nullptr;
        for (const Member &m : obj->members) {
            if (m.key == key) {
                found = &m;  // the last one wins, as in the JSON parser
            }
        }
        if (found && depth + 1 == path.size()) {
            QByteArray out = text;
            out.replace(found->valueStart, found->valueEnd - found->valueStart, serialized(value));
            return out;
        }
        if (found) {
            s.i = found->valueStart;
            continue;
        }
        // Not there: insert the member (and the objects it needs) here.
        QJsonValue v = value;
        for (int k = int(path.size()) - 1; k > depth; --k) {
            v = QJsonObject{{path.at(k), v}};
        }
        const QByteArray member = serialized(QJsonValue(key)) + ": " + inlineJson(v);
        QByteArray out = text;
        if (obj->members.isEmpty()) {
            const QByteArray inside = text.mid(obj->open + 1, obj->close - obj->open - 1);
            if (inside.trimmed().isEmpty()) {
                out.replace(obj->open + 1, inside.size(), " " + member + " ");
                return out;
            }
            // Only comments inside (options commented out): keep them, add the
            // member after them.
            const auto closeIndent = lineIndent(text, obj->close);
            if (!closeIndent) {
                out.insert(obj->close, member + " ");  // { /* note */ "k": v }
                return out;
            }
            qsizetype lineStart = obj->close - closeIndent->size();
            // The indent of the comment lines, else the brace's plus four spaces.
            QByteArray indent = *closeIndent + "    ";
            for (qsizetype i = obj->open + 1; i < obj->close; ++i) {
                if (text.at(i) == '\n') {
                    qsizetype j = i + 1;
                    while (j < obj->close && (text.at(j) == ' ' || text.at(j) == '\t')) {
                        ++j;
                    }
                    if (j < obj->close && text.at(j) == '/') {
                        indent = text.mid(i + 1, j - i - 1);
                        break;
                    }
                }
            }
            out.insert(lineStart, indent + member + nl);
            return out;
        }
        const Member &last = obj->members.last();
        Scanner after{text};
        after.i = last.valueEnd;
        after.ws();
        const bool trailingComma = after.at(',');  // kept: the new member gets one too
        const qsizetype tail = trailingComma ? after.i + 1 : last.valueEnd;
        const auto indent = lineIndent(text, last.keyStart);
        if (!indent) {
            out.insert(last.valueEnd, ", " + member);  // members on one line
            return out;
        }
        // The rest of the last member's line (a comment) stays with it: the
        // new member starts on the next line outside any comment.
        qsizetype eol = tail;
        while (eol < obj->close && text.at(eol) != '\n') {
            if (text.at(eol) == '/' && eol + 1 < text.size() && text.at(eol + 1) == '/') {
                eol = text.indexOf('\n', eol);
                break;
            }
            if (text.at(eol) == '/' && eol + 1 < text.size() && text.at(eol + 1) == '*') {
                const qsizetype end = text.indexOf("*/", eol + 2);
                eol = end < 0 ? obj->close : end + 2;
                continue;
            }
            ++eol;
        }
        const QByteArray line = nl + *indent + member + (trailingComma ? "," : "");
        if (eol > 0 && eol < obj->close && text.at(eol - 1) == '\r') {
            --eol;  // before the CR of a CRLF
        }
        if (eol < 0 || eol >= obj->close) {
            out.insert(tail, (trailingComma ? QByteArray() : QByteArray(",")) + line);  // closes on this line
            return out;
        }
        out.insert(eol, line);
        if (!trailingComma) {
            out.insert(last.valueEnd, ",");
        }
        return out;
    }
    return fail(QStringLiteral("not found"));
}

QJsonValue jsoncValue(const QByteArray &text, const QStringList &path)
{
    QJsonParseError pe;
    const QJsonDocument doc = QJsonDocument::fromJson(stripJsonComments(text), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isObject()) {
        return QJsonValue(QJsonValue::Undefined);
    }
    QJsonValue v = doc.object();
    for (const QString &k : path) {
        if (!v.isObject() || !v.toObject().contains(k)) {
            return QJsonValue(QJsonValue::Undefined);
        }
        v = v.toObject().value(k);
    }
    return v;
}

// ---------------------------------------------------------------------------------
// set / get on a file
// ---------------------------------------------------------------------------------

QJsonObject OptionChange::toJson() const
{
    QJsonObject o{{QStringLiteral("ok"), ok}, {QStringLiteral("key"), key}, {QStringLiteral("path"), path}};
    if (!ok) {
        o.insert(QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), code}, {QStringLiteral("message"), message}});
        return o;
    }
    o.insert(QStringLiteral("old"), oldValue.isUndefined() ? QJsonValue() : oldValue);
    o.insert(QStringLiteral("new"), newValue);
    o.insert(QStringLiteral("changed"), changed);
    o.insert(QStringLiteral("hash"), hash);
    o.insert(QStringLiteral("backup"), backup);
    return o;
}

namespace {
QByteArray defaultConfigText()
{
    QFile f(QStringLiteral(":/control-surface/config.example.jsonc"));
    return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray("{\"profiles\": []}\n");
}

QJsonValue defaultFor(const OptionSpec &spec)
{
    const Config d;
    if (spec.key == QLatin1String("input")) return d.device.input;
    if (spec.key == QLatin1String("serial")) return d.device.serial;
    if (spec.key == QLatin1String("cheatsheet.opacity")) return d.cheatsheet.opacity;
    if (spec.key == QLatin1String("cheatsheet.autoHideMs")) return d.cheatsheet.effectiveAutoHideMs();
    if (spec.key == QLatin1String("cheatsheet.position")) return d.cheatsheet.position;
    if (spec.key == QLatin1String("cheatsheet.eww")) return QJsonValue();  // unset: run --eww* decides
    if (spec.key == QLatin1String("settings.accelFactor")) return d.settings.accelFactor;
    if (spec.key == QLatin1String("settings.accelWindowMs")) return d.settings.accelWindowMs;
    if (spec.key == QLatin1String("settings.keyRateHz")) return d.settings.keyRateHz;
    return QJsonValue();
}

QString knownKeys()
{
    QStringList k;
    for (const OptionSpec &s : settableOptions()) {
        k << s.key;
    }
    return k.join(QStringLiteral(", "));
}
} // namespace

OptionChange setOption(const QString &configPath, const QString &key, const QString &valueText)
{
    OptionChange r;
    r.key = key;
    r.path = configPath;
    auto fail = [&r](const QString &code, const QString &message) {
        r.ok = false;
        r.code = code;
        r.message = message;
        return r;
    };
    const auto spec = optionSpec(key);
    if (!spec) {
        return fail(QStringLiteral("unknown-option"), QStringLiteral("no option '%1' (%2)").arg(key, knownKeys()));
    }
    r.key = spec->key;
    QString err;
    const auto value = parseOptionValue(*spec, valueText, &err);
    if (!value) {
        return fail(QStringLiteral("invalid-value"), err);
    }
    const ConfigStore store(configPath);
    const auto snap = store.read();
    const QByteArray text = snap.exists ? snap.text : defaultConfigText();
    QStringList path = spec->path;
    r.oldValue = jsoncValue(text, path);
    r.newValue = *value;
    if (spec->key == QLatin1String("cheatsheet.eww") && r.oldValue.isObject()) {
        // {"window": ..., "config": ...}: switch it with "enabled" (default
        // true) and keep the rest.
        r.oldValue = r.oldValue.toObject().value(QStringLiteral("enabled")).toBool(true);
        path << QStringLiteral("enabled");
    }
    if (r.oldValue == *value) {
        r.ok = true;
        r.hash = snap.hash;
        r.text = text;
        return r;  // already so; nothing written
    }
    const auto edited = setJsoncValue(text, path, *value, &err);
    if (!edited) {
        return fail(QStringLiteral("invalid-config"), err);
    }
    const auto v = store.validate(*edited);
    if (!v.ok) {
        return fail(QStringLiteral("invalid-config"), v.errors.join(QStringLiteral("; ")));
    }
    const auto w = store.write(*edited, snap.exists ? snap.hash : QString());
    if (!w.ok) {
        return fail(w.error, w.message);
    }
    r.ok = true;
    r.changed = true;
    r.hash = w.hash;
    r.backup = w.backup;
    r.text = *edited;
    return r;
}

QJsonObject getOption(const QString &configPath, const QString &key)
{
    const auto spec = optionSpec(key);
    if (!spec) {
        return QJsonObject{{QStringLiteral("ok"), false},
                           {QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), QStringLiteral("unknown-option")},
                                                                 {QStringLiteral("message"), QStringLiteral("no option '%1' (%2)").arg(key, knownKeys())}}}};
    }
    const auto snap = ConfigStore(configPath).read();
    const QJsonValue inFile = jsoncValue(snap.exists ? snap.text : defaultConfigText(), spec->path);
    QJsonObject o = spec->toJson();
    o.insert(QStringLiteral("ok"), true);
    o.insert(QStringLiteral("value"), inFile.isUndefined() ? QJsonValue() : inFile);  // null: not set
    o.insert(QStringLiteral("default"), defaultFor(*spec));
    QJsonValue effective = inFile.isUndefined() ? defaultFor(*spec) : inFile;
    if (spec->key == QLatin1String("cheatsheet.eww") && inFile.isObject()) {
        effective = inFile.toObject().value(QStringLiteral("enabled")).toBool(true);
    }
    o.insert(QStringLiteral("effective"), effective);
    o.insert(QStringLiteral("file"), snap.exists ? configPath : QStringLiteral("(built-in default)"));
    return o;
}

} // namespace cs
