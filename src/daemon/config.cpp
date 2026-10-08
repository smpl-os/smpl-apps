// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"

#include <QSet>
#include "keysink.h"
#include "kdenlivecontract.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QStandardPaths>
#include <algorithm>
#include <linux/input-event-codes.h>

namespace cs {

QString Binding::describe() const
{
    switch (kind) {
    case None:
        return QStringLiteral("none");
    case Keys: {
        QStringList l;
        for (const auto &k : keys) {
            l << chordName(k);
        }
        return QStringLiteral("keys:") + l.join(QLatin1Char(' '));
    }
    case Action:
        return QStringLiteral("action:") + name;
    case Control:
        return QStringLiteral("control:%1x%2").arg(name).arg(scale);
    case Command:
        return QStringLiteral("command:") + argv.join(QLatin1Char(' '));
    case Cycle:
        return QStringLiteral("cycle:") + name;
    case Request:
        return QStringLiteral("request:") + name;
    case Mouse:
        return QStringLiteral("mouse:") + name;
    case Cheatsheet:
        return QStringLiteral("cheatsheet:") + name;
    }
    return {};
}

QByteArray stripJsonComments(const QByteArray &in)
{
    QByteArray out;
    out.reserve(in.size());
    bool inString = false;
    for (qsizetype i = 0; i < in.size(); ++i) {
        const char c = in.at(i);
        const char n = i + 1 < in.size() ? in.at(i + 1) : '\0';
        if (inString) {
            out.append(c);
            if (c == '\\' && i + 1 < in.size()) {
                out.append(n);
                ++i;
            } else if (c == '"') {
                inString = false;
            }
            continue;
        }
        if (c == '"') {
            inString = true;
            out.append(c);
        } else if (c == '/' && n == '/') {
            while (i < in.size() && in.at(i) != '\n') {
                ++i;
            }
            out.append('\n');
        } else if (c == '/' && n == '*') {
            i += 2;
            while (i + 1 < in.size() && !(in.at(i) == '*' && in.at(i + 1) == '/')) {
                ++i;
            }
            ++i;
        } else {
            out.append(c);
        }
    }
    // Drop trailing commas before } or ] (outside strings).
    QByteArray clean;
    clean.reserve(out.size());
    inString = false;
    for (qsizetype i = 0; i < out.size(); ++i) {
        const char c = out.at(i);
        if (inString) {
            clean.append(c);
            if (c == '\\' && i + 1 < out.size()) {
                clean.append(out.at(++i));
            } else if (c == '"') {
                inString = false;
            }
            continue;
        }
        if (c == '"') {
            inString = true;
        } else if (c == ',') {
            qsizetype j = i + 1;
            while (j < out.size() && QChar::isSpace(uchar(out.at(j)))) {
                ++j;
            }
            if (j < out.size() && (out.at(j) == '}' || out.at(j) == ']')) {
                continue;
            }
        }
        clean.append(c);
    }
    return clean;
}

QString expandHome(const QString &path)
{
    if (path.startsWith(QLatin1String("~/"))) {
        return QDir::homePath() + path.mid(1);
    }
    return path;
}

QString defaultConfigPath()
{
    return QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation) + QStringLiteral("/control-surface/config.jsonc");
}

QString defaultHardwareMapPath()
{
    return QStandardPaths::writableLocation(QStandardPaths::GenericConfigLocation) + QStringLiteral("/control-surface/hardware-map.json");
}

namespace {
bool fail(QString *error, const QString &msg)
{
    if (error) {
        *error = msg;
    }
    return false;
}

std::optional<QList<KeyChord>> parseKeys(const QJsonValue &v, QString *error)
{
    QList<KeyChord> out;
    if (v.isString()) {
        return parseChordSequence(v.toString(), error);
    }
    if (v.isArray()) {
        for (const auto &e : v.toArray()) {
            auto seq = parseChordSequence(e.toString(), error);
            if (!seq) {
                return std::nullopt;
            }
            out << *seq;
        }
        if (!out.isEmpty()) {
            return out;
        }
    }
    if (error) {
        *error = QStringLiteral("\"keys\" must be a string or a list of strings");
    }
    return std::nullopt;
}

bool parseBindings(const QJsonObject &o, BindingMap &into, QString *error)
{
    for (auto it = o.begin(); it != o.end(); ++it) {
        const QString slot = it.key();
        if (slot.startsWith(QLatin1String("knob")) && !slot.contains(QLatin1Char('.')) && it.value().isObject()
            && !it.value().toObject().contains(QStringLiteral("keys")) && !it.value().toObject().contains(QStringLiteral("action"))
            && !it.value().toObject().contains(QStringLiteral("control"))) {
            const QJsonObject sub = it.value().toObject();
            for (auto s = sub.begin(); s != sub.end(); ++s) {
                static const QStringList events{QStringLiteral("turn"), QStringLiteral("ccw"), QStringLiteral("cw"), QStringLiteral("press")};
                if (s.key() == QLatin1String("shift") && s.value().isObject()) {
                    // knobN.shift: turning while the knob is held down
                    const QJsonObject shift = s.value().toObject();
                    for (auto e = shift.begin(); e != shift.end(); ++e) {
                        if (e.key() == QLatin1String("press") || !events.contains(e.key())) {
                            return fail(error, QStringLiteral("%1.shift: unknown event '%2' (turn, ccw or cw)").arg(slot, e.key()));
                        }
                        auto b = parseBinding(e.value(), error);
                        if (!b) {
                            if (error) {
                                *error = slot + QStringLiteral(".shift.") + e.key() + QStringLiteral(": ") + *error;
                            }
                            return false;
                        }
                        into.insert(slot + QStringLiteral(".shift.") + e.key(), *b);
                    }
                    continue;
                }
                if (!events.contains(s.key())) {
                    return fail(error, QStringLiteral("%1: unknown knob event '%2' (turn, ccw, cw, press or shift)").arg(slot, s.key()));
                }
                auto b = parseBinding(s.value(), error);
                if (!b) {
                    if (error) {
                        *error = slot + QLatin1Char('.') + s.key() + QStringLiteral(": ") + *error;
                    }
                    return false;
                }
                into.insert(slot + QLatin1Char('.') + s.key(), *b);
            }
            continue;
        }
        static const QRegularExpression slotRe(QStringLiteral("^(key([1-9]|1[0-6])|knob[1-3]\\.(turn|ccw|cw|press|shift\\.(turn|ccw|cw)))$"));
        if (!slotRe.match(slot).hasMatch()) {
            return fail(error, QStringLiteral("unknown control slot '%1' (key1..key16, knob1..knob3 with turn/ccw/cw/press/shift)").arg(slot));
        }
        auto b = parseBinding(it.value(), error);
        if (!b) {
            if (error) {
                *error = slot + QStringLiteral(": ") + *error;
            }
            return false;
        }
        into.insert(slot, *b);
    }
    return true;
}
} // namespace

std::optional<Binding> parseBinding(const QJsonValue &v, QString *error)
{
    Binding b;
    if (v.isNull()) {
        return b;
    }
    if (v.isString()) {
        const QString s = v.toString();
        const int colon = s.indexOf(QLatin1Char(':'));
        const QString kind = colon > 0 ? s.left(colon) : QString();
        const QString rest = colon > 0 ? s.mid(colon + 1) : s;
        if (s == QLatin1String("none")) {
            return b;
        }
        if (kind == QLatin1String("action")) {
            b.kind = Binding::Action;
            b.name = rest;
        } else if (kind == QLatin1String("cycle")) {
            b.kind = Binding::Cycle;
            b.name = rest;
        } else if (kind == QLatin1String("control")) {
            b.kind = Binding::Control;
            b.name = rest;
        } else {
            auto keys = parseChordSequence(rest, error);
            if (!keys) {
                return std::nullopt;
            }
            b.kind = Binding::Keys;
            b.keys = *keys;
        }
        return b;
    }
    if (!v.isObject()) {
        if (error) {
            *error = QStringLiteral("binding must be a string or an object");
        }
        return std::nullopt;
    }
    const QJsonObject o = v.toObject();
    b.label = o.value(QStringLiteral("label")).toString();
    if (o.contains(QStringLiteral("icon"))) {
        b.icon = o.value(QStringLiteral("icon")).toString();
        if (!isIconName(b.icon)) {
            if (error) {
                *error = QStringLiteral("\"icon\" must be a Tabler outline icon name like \"player-play\", or \"none\"");
            }
            return std::nullopt;
        }
    }
    if (o.contains(QStringLiteral("ifInstalled"))) {
        const QJsonValue need = o.value(QStringLiteral("ifInstalled"));
        const QJsonArray list = need.isArray() ? need.toArray() : QJsonArray{need};
        for (const QJsonValue &n : list) {
            const QString name = n.toString();
            if (!n.isString() || name.isEmpty() || name.contains(QLatin1Char(' '))) {
                if (error) {
                    *error = QStringLiteral("\"ifInstalled\" must be a program or desktop id, or a list of them");
                }
                return std::nullopt;
            }
            b.ifInstalled << name;
        }
    }
    b.targetFrom = o.value(QStringLiteral("targetFrom")).toString();
    b.options = o.value(QStringLiteral("options")).toObject().toVariantMap();
    if (o.contains(QStringLiteral("fallback"))) {
        auto fb = parseKeys(o.value(QStringLiteral("fallback")), error);
        if (!fb) {
            return std::nullopt;
        }
        b.keys = *fb;
    }
    if (o.contains(QStringLiteral("keys"))) {
        auto keys = parseKeys(o.value(QStringLiteral("keys")), error);
        if (!keys) {
            return std::nullopt;
        }
        b.kind = Binding::Keys;
        b.keys = *keys;
    } else if (o.contains(QStringLiteral("action"))) {
        b.kind = Binding::Action;
        b.name = o.value(QStringLiteral("action")).toString();
    } else if (o.contains(QStringLiteral("control"))) {
        b.kind = Binding::Control;
        b.name = o.value(QStringLiteral("control")).toString();
        b.scale = o.value(QStringLiteral("scale")).toDouble(1.0);
        b.accel = o.value(QStringLiteral("accel")).toDouble(0);
        if (b.scale <= 0 || b.accel < 0) {
            if (error) {
                *error = QStringLiteral("\"scale\" must be > 0 and \"accel\" >= 0");
            }
            return std::nullopt;
        }
    } else if (o.contains(QStringLiteral("command"))) {
        b.kind = Binding::Command;
        for (const auto &a : o.value(QStringLiteral("command")).toArray()) {
            const QString arg = a.toString();
            b.argv << (arg == QLatin1String("~") ? QDir::homePath() : expandHome(arg));
        }
        if (b.argv.isEmpty()) {
            if (error) {
                *error = QStringLiteral("\"command\" must be a non-empty argv list");
            }
            return std::nullopt;
        }
    } else if (o.contains(QStringLiteral("cycle"))) {
        b.kind = Binding::Cycle;
        b.name = o.value(QStringLiteral("cycle")).toString();
    } else if (o.contains(QStringLiteral("mouse"))) {
        b.kind = Binding::Mouse;
        b.name = o.value(QStringLiteral("mouse")).toString();
        if (!mouseActionNames().contains(b.name)) {
            if (error) {
                *error = QStringLiteral("unknown mouse action '%1' (%2)").arg(b.name, mouseActionNames().join(QStringLiteral(", ")));
            }
            return std::nullopt;
        }
    } else if (o.contains(QStringLiteral("cheatsheet"))) {
        b.kind = Binding::Cheatsheet;
        b.name = o.value(QStringLiteral("cheatsheet")).toString();
        if (b.name != QLatin1String("toggle") && b.name != QLatin1String("hold")) {
            if (error) {
                *error = QStringLiteral("\"cheatsheet\" must be \"toggle\" or \"hold\"");
            }
            return std::nullopt;
        }
    } else if (o.contains(QStringLiteral("request"))) {
        b.kind = Binding::Request;
        b.name = o.value(QStringLiteral("request")).toString();
        b.options = o.value(QStringLiteral("params")).toObject().toVariantMap();
    } else {
        if (error) {
            *error = QStringLiteral("binding object needs one of keys/mouse/action/control/command/cycle/request/cheatsheet");
        }
        return std::nullopt;
    }
    if ((b.kind == Binding::Action || b.kind == Binding::Control || b.kind == Binding::Cycle || b.kind == Binding::Request) && b.name.isEmpty()) {
        if (error) {
            *error = QStringLiteral("binding name is empty");
        }
        return std::nullopt;
    }
    return b;
}

bool isIconName(const QString &name)
{
    static const QRegularExpression re(QStringLiteral("^[a-z0-9]+(-[a-z0-9]+)*$"));
    return re.match(name).hasMatch();
}

bool Profile::matches(const QString &cls, const QString &title) const
{
    if (!hasMatch) {
        return true;
    }
    if (matchClass.isValid() && !matchClass.pattern().isEmpty() && !matchClass.match(cls).hasMatch()) {
        return false;
    }
    if (matchTitle.isValid() && !matchTitle.pattern().isEmpty() && !matchTitle.match(title).hasMatch()) {
        return false;
    }
    return true;
}

const Profile *Config::profileFor(const QString &cls, const QString &title) const
{
    for (const auto &p : profiles) {
        if (p.hasMatch && p.matches(cls, title)) {
            return &p;
        }
    }
    return globalProfile();
}

const Profile *Config::globalProfile() const
{
    for (const auto &p : profiles) {
        if (!p.hasMatch) {
            return &p;
        }
    }
    return nullptr;
}

std::optional<Config> parseConfig(const QByteArray &jsonc, const QString &baseDir, QString *error)
{
    QJsonParseError pe;
    const QJsonDocument doc = QJsonDocument::fromJson(stripJsonComments(jsonc), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isObject()) {
        if (error) {
            *error = QStringLiteral("JSON error at offset %1: %2").arg(pe.offset).arg(pe.errorString());
        }
        return std::nullopt;
    }
    const QJsonObject root = doc.object();
    Config cfg;
    const QJsonObject dev = root.value(QStringLiteral("device")).toObject();
    // vendor/product may be stated for documentation but must be the CH552 pad:
    // a different id would make the daemon grab (and swallow) another device.
    const QString vendor = dev.value(QStringLiteral("vendor")).toString(cfg.device.vendor).toLower();
    const QString product = dev.value(QStringLiteral("product")).toString(cfg.device.product).toLower();
    if (vendor != cfg.device.vendor || product != cfg.device.product) {
        if (error) {
            *error = QStringLiteral("device %1:%2 refused: this daemon only drives the 1189:8890 pad").arg(vendor, product);
        }
        return std::nullopt;
    }
    cfg.device.serial = dev.value(QStringLiteral("serial")).toString();
    cfg.device.input = dev.value(QStringLiteral("input")).toString(QStringLiteral("auto"));
    if (!QStringList{QStringLiteral("auto"), QStringLiteral("evdev"), QStringLiteral("raw")}.contains(cfg.device.input)) {
        if (error) {
            *error = QStringLiteral("device.input must be auto, evdev or raw");
        }
        return std::nullopt;
    }

    const QJsonValue sheet = root.value(QStringLiteral("cheatsheet"));
    if (sheet.isObject()) {
        const QJsonObject o = sheet.toObject();
        for (auto it = o.begin(); it != o.end(); ++it) {
            if (!QStringList{QStringLiteral("opacity"), QStringLiteral("autoHideMs"), QStringLiteral("position"), QStringLiteral("eww")}.contains(it.key())) {
                if (error) {
                    *error = QStringLiteral("cheatsheet: unknown option '%1' (opacity, autoHideMs, position, eww)").arg(it.key());
                }
                return std::nullopt;
            }
        }
        const QJsonValue eww = o.value(QStringLiteral("eww"));
        if (eww.isBool()) {
            cfg.cheatsheet.eww.enabled = eww.toBool();
        } else if (eww.isObject()) {
            const QJsonObject e = eww.toObject();
            static const QStringList known{QStringLiteral("enabled"), QStringLiteral("variable"), QStringLiteral("window"), QStringLiteral("binary"), QStringLiteral("config")};
            for (auto it = e.begin(); it != e.end(); ++it) {
                if (!known.contains(it.key()) || (it.key() == QLatin1String("enabled") ? !it->isBool() : !it->isString())) {
                    if (error) {
                        *error = QStringLiteral("cheatsheet.eww: '%1' is not an option (enabled: bool; variable, window, binary, config: strings)").arg(it.key());
                    }
                    return std::nullopt;
                }
            }
            EwwConfig &h = cfg.cheatsheet.eww;
            h.enabled = e.value(QStringLiteral("enabled")).toBool(true);
            auto field = [&e](const char *key, std::optional<QString> &out) {
                if (e.contains(QLatin1String(key))) {
                    out = e.value(QLatin1String(key)).toString();
                }
            };
            field("variable", h.variable);
            field("window", h.window);
            field("binary", h.binary);
            field("config", h.configDir);
            if (h.configDir) {
                h.configDir = expandHome(*h.configDir);
            }
            static const QRegularExpression name(QStringLiteral("^[A-Za-z_][A-Za-z0-9_-]*$"));
            if ((h.variable && !name.match(*h.variable).hasMatch()) || (h.window && !h.window->isEmpty() && !name.match(*h.window).hasMatch())
                || (h.binary && h.binary->isEmpty())) {
                if (error) {
                    *error = QStringLiteral("cheatsheet.eww: variable and window are eww names (letters, digits, _ and -), binary must not be empty");
                }
                return std::nullopt;
            }
        } else if (!eww.isUndefined() && !eww.isNull()) {
            if (error) {
                *error = QStringLiteral("cheatsheet.eww: true, false or {variable, window, binary, config}");
            }
            return std::nullopt;
        }
        cfg.cheatsheet.opacity = o.value(QStringLiteral("opacity")).toDouble(cfg.cheatsheet.opacity);
        if (o.contains(QStringLiteral("autoHideMs"))) {
            cfg.cheatsheet.autoHideMs = o.value(QStringLiteral("autoHideMs")).toInt(-1);
        }
        cfg.cheatsheet.position = o.value(QStringLiteral("position")).toString(cfg.cheatsheet.position);
        if (!(cfg.cheatsheet.opacity >= 0.05 && cfg.cheatsheet.opacity <= 1.0) || (cfg.cheatsheet.autoHideMs && (*cfg.cheatsheet.autoHideMs < 0 || *cfg.cheatsheet.autoHideMs > 600000))
            || !CheatsheetOptions::positions().contains(cfg.cheatsheet.position)) {
            if (error) {
                *error = QStringLiteral("cheatsheet: opacity 0.05..1, autoHideMs 0..600000 (0 = until hidden; unset = %1), position one of %2").arg(CheatsheetOptions::kDefaultAutoHideMs).arg(CheatsheetOptions::positions().join(QStringLiteral(", ")));
            }
            return std::nullopt;
        }
    } else if (!sheet.isUndefined() && !sheet.isNull()) {
        if (error) {
            *error = QStringLiteral("cheatsheet: must be an object");
        }
        return std::nullopt;
    }

    const QJsonValue layout = root.value(QStringLiteral("layout"));
    if (layout.isString()) {
        auto p = builtinBoardProfile(layout.toString());
        if (!p) {
            QStringList ids;
            for (const BoardProfile &b : builtinBoardProfiles()) {
                ids << b.id;
            }
            if (error) {
                *error = QStringLiteral("layout: unknown board profile '%1' (known: %2)").arg(layout.toString(), ids.join(QStringLiteral(", ")));
            }
            return std::nullopt;
        }
        p->source = QStringLiteral("config");
        cfg.layout = *p;
    } else if (layout.isObject()) {
        const QJsonObject l = layout.toObject();
        const int keys = l.value(QStringLiteral("keys")).toInt(-1);
        const int knobs = l.value(QStringLiteral("knobs")).toInt(0);
        const int columns = l.value(QStringLiteral("columns")).toInt(keys >= 10 ? 5 : qMax(1, qMin(keys, 4)));
        if (keys < 0 || keys > 16 || knobs < 0 || knobs > 3 || keys + knobs == 0 || columns < 1 || columns > 8) {
            if (error) {
                *error = QStringLiteral("layout: needs \"keys\" 0..16, \"knobs\" 0..3 (at least one input) and \"columns\" 1..8");
            }
            return std::nullopt;
        }
        BoardProfile p = gridProfile(QStringLiteral("custom-%1k%2e").arg(keys).arg(knobs), QStringLiteral("%1 keys, %2 knobs").arg(keys).arg(knobs), keys, knobs, columns);
        p.source = QStringLiteral("config");
        cfg.layout = p;
    } else if (!layout.isUndefined() && !layout.isNull()) {
        if (error) {
            *error = QStringLiteral("layout: must be a board profile id or an object");
        }
        return std::nullopt;
    }

    const QJsonValue hw = root.value(QStringLiteral("hardware"));
    if (hw.isString()) {
        const QString s = hw.toString();
        if (s == QLatin1String("default:keys-then-knobs")) {
            cfg.hardware = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
            cfg.hardwareSource = s;
        } else if (s == QLatin1String("default:vendor-twelve")) {
            cfg.hardware = HardwareMap::fromScheme(ch552::Numbering::VendorTwelve);
            cfg.hardwareSource = s;
        } else {
            QString path = expandHome(s);
            if (QFileInfo(path).isRelative()) {
                path = baseDir + QLatin1Char('/') + path;
            }
            QFile f(path);
            if (f.open(QIODevice::ReadOnly)) {
                const auto hdoc = QJsonDocument::fromJson(stripJsonComments(f.readAll()));
                auto m = HardwareMap::fromJson(hdoc.object(), error);
                if (!m) {
                    return std::nullopt;
                }
                cfg.hardware = *m;
                cfg.hardwareSource = path;
            } else {
                // A missing learned map is not fatal: the flashed default applies until verify-pad writes one.
                cfg.hardwareSource = QStringLiteral("default:keys-then-knobs (missing %1)").arg(path);
            }
        }
    } else if (hw.isObject()) {
        auto m = HardwareMap::fromJson(hw.toObject(), error);
        if (!m) {
            return std::nullopt;
        }
        cfg.hardware = *m;
        cfg.hardwareSource = QStringLiteral("inline");
    }

    const QJsonObject st = root.value(QStringLiteral("settings")).toObject();
    cfg.settings.coalesceMs = st.value(QStringLiteral("coalesceMs")).toInt(cfg.settings.coalesceMs);
    cfg.settings.ackTimeoutMs = st.value(QStringLiteral("ackTimeoutMs")).toInt(cfg.settings.ackTimeoutMs);
    cfg.settings.accelWindowMs = st.value(QStringLiteral("accelWindowMs")).toInt(cfg.settings.accelWindowMs);
    cfg.settings.accelFactor = st.value(QStringLiteral("accelFactor")).toDouble(cfg.settings.accelFactor);
    cfg.settings.keyRateHz = st.value(QStringLiteral("keyRateHz")).toInt(cfg.settings.keyRateHz);
    cfg.settings.gestureIdleMs = qBound(50, st.value(QStringLiteral("gestureIdleMs")).toInt(cfg.settings.gestureIdleMs), 590);

    for (const auto &pv : root.value(QStringLiteral("profiles")).toArray()) {
        const QJsonObject po = pv.toObject();
        Profile p;
        p.name = po.value(QStringLiteral("name")).toString();
        const QJsonObject match = po.value(QStringLiteral("match")).toObject();
        p.hasMatch = !match.isEmpty();
        p.matchClass = QRegularExpression(match.value(QStringLiteral("class")).toString());
        p.matchTitle = QRegularExpression(match.value(QStringLiteral("title")).toString());
        if (!p.matchClass.isValid() || !p.matchTitle.isValid()) {
            if (error) {
                *error = QStringLiteral("profile %1: bad match regex").arg(p.name);
            }
            return std::nullopt;
        }
        p.kdenlive = po.value(QStringLiteral("kdenlive")).toBool(false);
        p.fallthrough = po.value(QStringLiteral("fallthrough")).toBool(true);
        p.keyFallback = po.value(QStringLiteral("keyFallback")).toBool(false);
        const QJsonObject modes = po.value(QStringLiteral("modes")).toObject();
        for (auto it = modes.begin(); it != modes.end(); ++it) {
            QStringList values;
            for (const auto &v : it.value().toArray()) {
                values << v.toString();
            }
            if (values.isEmpty()) {
                if (error) {
                    *error = QStringLiteral("profile %1: mode %2 has no values").arg(p.name, it.key());
                }
                return std::nullopt;
            }
            p.modes.insert(it.key(), values);
        }
        QString err;
        if (!parseBindings(po.value(QStringLiteral("bindings")).toObject(), p.bindings, &err)) {
            if (error) {
                *error = QStringLiteral("profile %1: %2").arg(p.name, err);
            }
            return std::nullopt;
        }
        for (const auto &lv : po.value(QStringLiteral("layers")).toArray()) {
            const QJsonObject lo = lv.toObject();
            Layer l;
            l.name = lo.value(QStringLiteral("name")).toString();
            l.when = lo.value(QStringLiteral("when")).toObject().toVariantMap();
            if (l.when.contains(QStringLiteral("held"))) {
                const auto held = parseHeldCondition(lo.value(QStringLiteral("when")).toObject().value(QStringLiteral("held")), &err);
                if (!held) {
                    if (error) {
                        *error = QStringLiteral("profile %1 layer %2: %3").arg(p.name, l.name, err);
                    }
                    return std::nullopt;
                }
                l.held = *held;
                l.when.remove(QStringLiteral("held"));
                for (const QStringList &set : *held) {
                    for (const QString &c : set) {
                        p.heldControls.insert(c);
                    }
                }
            }
            if (!parseBindings(lo.value(QStringLiteral("bindings")).toObject(), l.bindings, &err)) {
                if (error) {
                    *error = QStringLiteral("profile %1 layer %2: %3").arg(p.name, l.name, err);
                }
                return std::nullopt;
            }
            p.layers << l;
        }
        cfg.profiles << p;
    }
    QString checkError;
    if (!checkConfig(cfg, &checkError)) {
        if (error) {
            *error = checkError;
        }
        return std::nullopt;
    }
    return cfg;
}

bool checkConfig(Config &cfg, QString *error)
{
    // Errors: references that can never work. Warnings: names this daemon does
    // not know (a newer Kdenlive may offer them) and bindings that do nothing.
    {
        // A cheatsheet goes on something pressed: a key or a knob press. "hold"
        // on a knob whose press waits for release (it has shift bindings) can
        // only toggle.
        static const QRegularExpression pressSlot(QStringLiteral("^(key\\d+|knob\\d+\\.press)$"));
        auto checkSheet = [&](const QString &where, const BindingMap &m, const BindingMap &profileBindings) -> bool {
            for (auto it = m.cbegin(); it != m.cend(); ++it) {
                if (it.value().kind != Binding::Cheatsheet) {
                    continue;
                }
                if (!pressSlot.match(it.key()).hasMatch()) {
                    return fail(error, QStringLiteral("%1 %2: a cheatsheet binding goes on a key or a knob press").arg(where, it.key()));
                }
                if (it.value().name == QLatin1String("hold") && it.key().startsWith(QLatin1String("knob"))) {
                    const QString knob = it.key().section(QLatin1Char('.'), 0, 0);
                    bool shifted = false;
                    for (const BindingMap *bm : {&m, &profileBindings}) {
                        for (auto k = bm->cbegin(); k != bm->cend() && !shifted; ++k) {
                            shifted = k.key().startsWith(knob + QStringLiteral(".shift."));
                        }
                    }
                    if (shifted) {
                        cfg.warnings << QStringLiteral("%1 %2: \"hold\" acts as \"toggle\" here, because %3 has shift bindings (its press fires on release)").arg(where, it.key(), knob);
                    }
                }
            }
            return true;
        };
        for (const Profile &p : cfg.profiles) {
            if (!checkSheet(QStringLiteral("profile %1").arg(p.name), p.bindings, p.bindings)) {
                return false;
            }
            for (const Layer &l : p.layers) {
                if (!checkSheet(QStringLiteral("profile %1 layer %2:").arg(p.name, l.name), l.bindings, p.bindings)) {
                    return false;
                }
            }
        }
    }
    if (cfg.layout) {
        // Bindings for inputs this pad does not have are harmless but likely a mistake.
        QSet<QString> present;
        for (const BoardKey &k : cfg.layout->keys) {
            present.insert(k.control);
        }
        for (const BoardKnob &k : cfg.layout->knobs) {
            present.insert(k.control);
        }
        auto checkSlots = [&](const QString &where, const BindingMap &m) {
            for (auto it = m.cbegin(); it != m.cend(); ++it) {
                const QString control = it.key().section(QLatin1Char('.'), 0, 0);
                if (!present.contains(control)) {
                    cfg.warnings << QStringLiteral("%1 %2: the %3 layout has no %4").arg(where, it.key(), cfg.layout->id, control);
                }
            }
        };
        for (const Profile &p : cfg.profiles) {
            checkSlots(QStringLiteral("profile %1").arg(p.name), p.bindings);
            for (const Layer &l : p.layers) {
                checkSlots(QStringLiteral("profile %1 layer %2:").arg(p.name, l.name), l.bindings);
            }
        }
    }
    const Profile *global = cfg.globalProfile();
    for (const Profile &p : cfg.profiles) {
        auto modeKnown = [&](const QString &m) { return p.modes.contains(m) || (global && global->modes.contains(m)); };
        auto checkBinding = [&](const QString &where, const Binding &b) -> bool {
            if (b.kind == Binding::Cycle && !modeKnown(b.name)) {
                return fail(error, QStringLiteral("%1: cycles undefined mode '%2' (define it under \"modes\")").arg(where, b.name));
            }
            if (b.kind == Binding::Control && b.name.startsWith(QLatin1Char('$')) && !modeKnown(b.name.mid(1))) {
                return fail(error, QStringLiteral("%1: control comes from undefined mode '%2'").arg(where, b.name.mid(1)));
            }
            for (auto it = b.options.cbegin(); it != b.options.cend(); ++it) {
                const QString v = it.value().toString();
                if (it.value().typeId() == QMetaType::QString && v.startsWith(QLatin1Char('$')) && !v.startsWith(QLatin1String("$ctx:"))
                    && !v.startsWith(QLatin1String("$!ctx:")) && !modeKnown(v.mid(1))) {
                    return fail(error, QStringLiteral("%1: option %2 uses undefined mode '%3'").arg(where, it.key(), v.mid(1)));
                }
            }
            const bool kdenliveOnly = b.kind == Binding::Action || b.kind == Binding::Control || b.kind == Binding::Request;
            if (kdenliveOnly && !p.kdenlive) {
                cfg.warnings << QStringLiteral("%1: %2 bindings only work in a profile with \"kdenlive\": true").arg(where, b.kind == Binding::Action ? QStringLiteral("action") : b.kind == Binding::Control ? QStringLiteral("control") : QStringLiteral("request"));
            }
            if (b.kind == Binding::Control && !b.name.startsWith(QLatin1Char('$')) && !contract::kKnownControls.contains(b.name)) {
                cfg.warnings << QStringLiteral("%1: unknown control '%2' (known: %3)").arg(where, b.name, contract::kKnownControls.join(QStringLiteral(", ")));
            }
            if (b.kind == Binding::Request && !contract::kKnownCommands.contains(b.name)) {
                cfg.warnings << QStringLiteral("%1: unknown command '%2' (known: %3)").arg(where, b.name, contract::kKnownCommands.join(QStringLiteral(", ")));
            }
            return true;
        };
        for (auto it = p.bindings.cbegin(); it != p.bindings.cend(); ++it) {
            if (!checkBinding(QStringLiteral("profile %1 %2").arg(p.name, it.key()), it.value())) {
                return false;
            }
        }
        for (const Layer &l : p.layers) {
            for (auto w = l.when.cbegin(); w != l.when.cend(); ++w) {
                if (w.key().startsWith(QLatin1String("$mode.")) && !modeKnown(w.key().mid(6))) {
                    return fail(error, QStringLiteral("profile %1 layer %2: condition uses undefined mode '%3'").arg(p.name, l.name, w.key().mid(6)));
                }
            }
            if (l.when.isEmpty() && l.held.isEmpty()) {
                cfg.warnings << QStringLiteral("profile %1 layer %2: no \"when\" condition, so it always applies").arg(p.name, l.name);
            }
            for (auto it = l.bindings.cbegin(); it != l.bindings.cend(); ++it) {
                if (!checkBinding(QStringLiteral("profile %1 layer %2 %3").arg(p.name, l.name, it.key()), it.value())) {
                    return false;
                }
            }
        }
        if (p.keyFallback && !p.kdenlive) {
            cfg.warnings << QStringLiteral("profile %1: \"keyFallback\" only applies to Kdenlive profiles").arg(p.name);
        }
    }
    return true;
}

std::optional<Config> loadConfig(const QString &path, QString *error)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        if (error) {
            *error = QStringLiteral("cannot read %1").arg(path);
        }
        return std::nullopt;
    }
    return parseConfig(f.readAll(), QFileInfo(path).absolutePath(), error);
}

QVariant valueAtPath(const QVariantMap &map, const QString &dottedPath)
{
    const QStringList parts = dottedPath.split(QLatin1Char('.'));
    QVariant cur = map;
    for (const auto &p : parts) {
        if (cur.typeId() != QMetaType::QVariantMap) {
            return {};
        }
        const QVariantMap m = cur.toMap();
        if (!m.contains(p)) {
            return {};
        }
        cur = m.value(p);
    }
    return cur;
}

namespace {
bool valueMatches(const QVariant &want, const QVariant &have)
{
    if (want.typeId() == QMetaType::QVariantList) {
        for (const auto &alt : want.toList()) {
            if (valueMatches(alt, have)) {
                return true;
            }
        }
        return false;
    }
    if (want.typeId() == QMetaType::Bool) {
        // true: present and truthy; false: absent or falsy
        const bool truthy = have.isValid() && !have.isNull()
            && (have.typeId() == QMetaType::Bool ? have.toBool()
                : have.typeId() == QMetaType::QString ? !have.toString().isEmpty()
                : have.typeId() == QMetaType::QVariantMap ? !have.toMap().isEmpty()
                : have.typeId() == QMetaType::QVariantList ? !have.toList().isEmpty()
                : have.typeId() == QMetaType::QStringList ? !have.toStringList().isEmpty()
                : true);
        return want.toBool() == truthy;
    }
    if (want.typeId() == QMetaType::QString) {
        const QString w = want.toString();
        if (w.startsWith(QLatin1Char('!'))) {
            return !valueMatches(w.mid(1), have);
        }
        if (!have.isValid()) {
            return false;
        }
        if (w.size() >= 2 && w.startsWith(QLatin1Char('/')) && w.endsWith(QLatin1Char('/'))) {
            return QRegularExpression(w.mid(1, w.size() - 2)).match(have.toString()).hasMatch();
        }
        return have.toString() == w;
    }
    if (!have.isValid()) {
        return false;
    }
    return want.toDouble() == have.toDouble();
}
} // namespace

std::optional<QList<QStringList>> parseHeldCondition(const QJsonValue &v, QString *error)
{
    static const QRegularExpression control(QStringLiteral("^(key([1-9]|1[0-6])|knob[1-3])$"));
    QStringList alternatives;
    if (v.isString()) {
        alternatives << v.toString();
    } else if (v.isArray() && !v.toArray().isEmpty()) {
        for (const auto &e : v.toArray()) {
            if (!e.isString()) {
                alternatives.clear();
                break;
            }
            alternatives << e.toString();
        }
    }
    if (alternatives.isEmpty()) {
        if (error) {
            *error = QStringLiteral("\"held\" takes a control (\"key1\"), a list of alternatives ([\"key1\", \"key13\"]) or controls held together (\"key1+knob3\")");
        }
        return std::nullopt;
    }
    QList<QStringList> out;
    for (const QString &a : std::as_const(alternatives)) {
        QStringList set;
        for (const QString &part : a.split(QLatin1Char('+'))) {
            const QString c = part.trimmed();
            if (!control.match(c).hasMatch()) {
                if (error) {
                    *error = QStringLiteral("\"held\": '%1' is not a key or knob (key1..key16, knob1..knob3; a knob means its press)").arg(c);
                }
                return std::nullopt;
            }
            if (!set.contains(c)) {
                set << c;
            }
        }
        out << set;
    }
    return out;
}

bool Layer::heldMatches(const QSet<QString> &down) const
{
    if (held.isEmpty()) {
        return true;
    }
    for (const QStringList &set : held) {
        if (std::all_of(set.cbegin(), set.cend(), [&down](const QString &c) { return down.contains(c); })) {
            return true;
        }
    }
    return false;
}

bool conditionMatches(const QVariantMap &when, const QVariantMap &context)
{
    for (auto it = when.begin(); it != when.end(); ++it) {
        if (!valueMatches(it.value(), valueAtPath(context, it.key()))) {
            return false;
        }
    }
    return true;
}

QJsonObject ConfigIssue::toJson() const
{
    auto orNull = [](const QString &v) { return v.isEmpty() ? QJsonValue() : QJsonValue(v); };
    return QJsonObject{{QStringLiteral("message"), message},
                       {QStringLiteral("profile"), orNull(profile)},
                       {QStringLiteral("layer"), orNull(layer)},
                       {QStringLiteral("slot"), orNull(slot)}};
}

ConfigIssue describeConfigIssue(const QString &text)
{
    // The texts come from parseConfig/checkConfig:
    //   "profile P: SLOT: msg", "profile P SLOT: msg", "profile P layer L: [SLOT: ]msg",
    //   "profile P: unknown control slot 'S' (...)", "profile P: msg".
    static const QString slot = QStringLiteral("(key\\d+|knob\\d+(?:\\.[a-z]+)*)");
    static const QRegularExpression withLayer(QStringLiteral("^profile (.+?) layer (.+?): (?:%1: )?(.*)$").arg(slot));
    static const QRegularExpression plainSlot(QStringLiteral("^profile (.+?): %1: (.*)$").arg(slot));
    static const QRegularExpression spaced(QStringLiteral("^profile (.+?) %1: (.*)$").arg(slot));
    static const QRegularExpression plain(QStringLiteral("^profile (.+?): (.*)$"));
    static const QRegularExpression quoted(QStringLiteral("unknown control slot '([^']*)'"));
    ConfigIssue i;
    i.message = text;
    QRegularExpressionMatch m = withLayer.match(text);
    if (m.hasMatch()) {
        i.profile = m.captured(1);
        i.layer = m.captured(2);
        i.slot = m.captured(3);
        return i;
    }
    for (const QRegularExpression *re : {&plainSlot, &spaced}) {
        m = re->match(text);
        if (m.hasMatch()) {
            i.profile = m.captured(1);
            i.slot = m.captured(2);
            return i;
        }
    }
    m = plain.match(text);
    if (m.hasMatch()) {
        i.profile = m.captured(1);
        const auto q = quoted.match(m.captured(2));
        if (q.hasMatch()) {
            i.slot = q.captured(1);
        }
    }
    return i;
}

QStringList hardwareControls(const Config &cfg)
{
    QStringList controls;
    for (const KeyChord &k : cfg.hardware.chords()) {
        if (auto t = cfg.hardware.lookup(k)) {
            controls << t->control;
        }
    }
    return controls;
}

BoardProfile effectiveLayout(const Config &cfg, const QString &firmwareBoard)
{
    if (cfg.layout) {
        return *cfg.layout;  // the user's explicit choice wins over the firmware
    }
    if (!firmwareBoard.isEmpty()) {
        if (auto p = builtinBoardProfile(firmwareBoard)) {
            p->source = QStringLiteral("firmware");
            return *p;
        }
    }
    // The built-in keys-then-knobs scheme (also when its learned map is still
    // missing) names nothing about this pad: that is the default.
    const bool learned = !cfg.hardwareSource.startsWith(QLatin1String("default:keys-then-knobs"));
    return profileForControls(hardwareControls(cfg), learned ? QStringLiteral("hardware-map") : QStringLiteral("default"));
}

namespace {
int firmwareSlotCount(const std::optional<BoardProfile> &board, int fwSlots)
{
    return fwSlots > 0 ? fwSlots : board ? board->slotCount() : 0;
}

QString shape(const BoardProfile &p)
{
    return QStringLiteral("%1 (%2 keys, %3 knobs = %4 slots)").arg(p.id).arg(p.keys.size()).arg(p.knobs.size()).arg(p.slotCount());
}
} // namespace

QJsonObject layoutReport(const BoardProfile &effective, const std::optional<BoardProfile> &firmwareBoard, int firmwareSlots)
{
    QJsonObject o = effective.toJson();
    const int fw = firmwareSlotCount(firmwareBoard, firmwareSlots);
    o.insert(QStringLiteral("firmwareLayout"), firmwareBoard ? QJsonValue(firmwareBoard->toJson()) : QJsonValue());
    o.insert(QStringLiteral("firmwareSlots"), fw > 0 ? QJsonValue(fw) : QJsonValue());
    o.insert(QStringLiteral("matchesFirmware"), fw > 0 ? QJsonValue(effective.slotCount() == fw) : QJsonValue());
    return o;
}

QString layoutMismatchWarning(const BoardProfile &effective, const std::optional<BoardProfile> &firmwareBoard, int firmwareSlots)
{
    const int fw = firmwareSlotCount(firmwareBoard, firmwareSlots);
    if (effective.source != QLatin1String("config") || fw <= 0 || effective.slotCount() == fw) {
        return {};
    }
    return QStringLiteral("layout: the config's %1 overrides the firmware's %2, which has %3 slots; "
                          "raw input stays off (the pad's keymap is used) and inputs outside the layout show nowhere")
        .arg(shape(effective), firmwareBoard ? firmwareBoard->id : QStringLiteral("board"))
        .arg(fw);
}

QStringList boardWarnings(const Config &cfg, const BoardProfile &layout)
{
    QStringList out;
    QHash<QString, BoardKnob> knobs;
    QSet<QString> present;
    for (const BoardKnob &k : layout.knobs) {
        knobs.insert(k.control, k);
        present.insert(k.control);
    }
    for (const BoardKey &k : layout.keys) {
        present.insert(k.control);
    }
    // The keymap chord of every input (evdev input mode), by event slot.
    QHash<QString, KeyChord> chordOf;
    for (const KeyChord &c : cfg.hardware.chords()) {
        if (const auto t = cfg.hardware.lookup(c)) {
            chordOf.insert(t->name(), c);
        }
    }
    const bool evdev = cfg.device.input == QLatin1String("evdev");
    auto events = [](const QString &slot) -> QStringList {
        const QString control = slot.section(QLatin1Char('.'), 0, 0);
        const QString rest = slot.section(QLatin1Char('.'), 1);
        if (rest.isEmpty()) {
            return {control};
        }
        if (rest == QLatin1String("turn")) {
            return {control + QStringLiteral(".ccw"), control + QStringLiteral(".cw")};
        }
        if (rest == QLatin1String("ccw") || rest == QLatin1String("cw") || rest == QLatin1String("press")) {
            return {slot};
        }
        return {};  // shift slots
    };
    auto check = [&](const QString &where, const BindingMap &m, const Layer *layer) {
        QSet<QString> said;
        for (auto it = m.cbegin(); it != m.cend(); ++it) {
            const QString knob = it.key().section(QLatin1Char('.'), 0, 0);
            if (it.key().contains(QLatin1String(".shift.")) && knobs.contains(knob) && !said.contains(knob)) {
                const BoardKnob &k = knobs[knob];
                QString why;
                if (k.pressPinsEncoder) {
                    why = QStringLiteral("pressing %1 holds one of its encoder lines low on this pad, so a turn while it is pressed has no direction").arg(knob);
                } else if (!layout.turnsWhilePressed) {
                    why = QStringLiteral("the pad's firmware ignores turns while a knob is pressed");
                }
                if (!why.isEmpty()) {
                    said.insert(knob);
                    out << QStringLiteral("%1 %2: never fires: %3 (and %4's press waits for its release). Hold a key and turn instead (\"when\": {\"held\": \"key1\"})")
                               .arg(where, it.key(), why, knob);
                }
            }
            if (!layer || !evdev) {
                continue;
            }
            for (const QStringList &set : layer->held) {
                for (const QString &h : set) {
                    const KeyChord hc = chordOf.value(h.startsWith(QLatin1String("knob")) ? h + QStringLiteral(".press") : h);
                    for (const QString &ev : events(it.key())) {
                        const KeyChord ec = chordOf.value(ev);
                        if (hc.isValid() && ec.isValid() && ev.section(QLatin1Char('.'), 0, 0) != h && hc.key == ec.key) {
                            out << QStringLiteral("%1 %2: with \"input\": \"evdev\", %3 held and %4 share the pad's %5 key: that input is lost and %3 reads as released. Use raw input: control-surfaced set input auto (raw on firmware 2.0.2+)")
                                       .arg(where, it.key(), h, ev, keyName(ec.key));
                        }
                    }
                }
            }
        }
    };
    for (const Profile &p : cfg.profiles) {
        check(QStringLiteral("profile %1").arg(p.name), p.bindings, nullptr);
        for (const Layer &l : p.layers) {
            const QString where = QStringLiteral("profile %1 layer %2:").arg(p.name, l.name);
            check(where, l.bindings, &l);
            QStringList missing;
            for (const QStringList &set : l.held) {
                for (const QString &h : set) {
                    if (!present.contains(h) && !missing.contains(h)) {
                        missing << h;
                    }
                }
            }
            if (!missing.isEmpty()) {
                out << QStringLiteral("%1 \"held\": the %2 layout has no %3").arg(where, layout.id, missing.join(QStringLiteral(", ")));
            }
            // The pad reads some controls one at a time (sy181: keys 2-15 and
            // the knob presses): pressing one reports the held one released.
            const QSet<QString> single(layout.oneAtATime.cbegin(), layout.oneAtATime.cend());
            const QString singleText = QStringLiteral("this pad reads keys 2-15 and the knob presses one at a time");
            QStringList heldSingle;
            for (const QStringList &set : l.held) {
                QStringList inSet;
                for (const QString &h : set) {
                    if (single.contains(h)) {
                        inSet << h;
                        if (!heldSingle.contains(h)) {
                            heldSingle << h;
                        }
                    }
                }
                if (inSet.size() > 1) {
                    out << QStringLiteral("%1 \"held\": %2 can never be held together: %3").arg(where, inSet.join(QLatin1Char('+')), singleText);
                }
            }
            for (auto it = l.bindings.cbegin(); it != l.bindings.cend() && !heldSingle.isEmpty(); ++it) {
                const QString control = it.key().section(QLatin1Char('.'), 0, 0);
                const bool pressed = !it.key().contains(QLatin1Char('.')) || it.key().endsWith(QLatin1String(".press"));
                QStringList others = heldSingle;
                others.removeAll(control);
                if (pressed && single.contains(control) && !others.isEmpty()) {
                    out << QStringLiteral("%1 %2: never fires: pressing %3 reports %4 released (%5). Bind knob turns or key1 in a layer held on %4")
                               .arg(where, it.key(), control, others.join(QStringLiteral(" or ")), singleText);
                }
            }
            // evdev: a held chord's modifiers stay in the pad's report, so an
            // input pressed meanwhile can read as another control.
            if (evdev) {
                QSet<QString> told;
                auto pressChord = [&chordOf](const QString &c) { return chordOf.value(c.startsWith(QLatin1String("knob")) ? c + QStringLiteral(".press") : c); };
                for (const QStringList &set : l.held) {
                    // Members of one held set on the same F-key: the second
                    // adds only its modifier to the report (key1 = F14 and
                    // key13 = Ctrl+F14), so they never read as held together.
                    for (int i = 0; i < set.size(); ++i) {
                        for (int j = i + 1; j < set.size(); ++j) {
                            const KeyChord a = pressChord(set[i]), b = pressChord(set[j]);
                            if (a.isValid() && b.isValid() && a.key == b.key) {
                                out << QStringLiteral("%1 \"held\": with \"input\": \"evdev\", %2 and %3 share the pad's %4 key, so they never read as held together. Use raw input: control-surfaced set input auto (raw on firmware 2.0.2+)")
                                           .arg(where, set[i], set[j], keyName(a.key));
                            }
                        }
                    }
                }
                for (const QStringList &set : l.held) {
                    for (const QString &h : set) {
                        const KeyChord hc = chordOf.value(h.startsWith(QLatin1String("knob")) ? h + QStringLiteral(".press") : h);
                        if (!hc.isValid() || !hc.mods) {
                            continue;
                        }
                        QStringList inputs;
                        for (auto it = l.bindings.cbegin(); it != l.bindings.cend(); ++it) {
                            inputs << events(it.key());
                        }
                        for (const QString &o : set) {
                            if (o != h) {
                                inputs << (o.startsWith(QLatin1String("knob")) ? o + QStringLiteral(".press") : o);
                            }
                        }
                        for (const QString &x : std::as_const(inputs)) {
                            const KeyChord xc = chordOf.value(x);
                            const KeyChord seen{quint8(xc.mods | hc.mods), xc.key};
                            const QString xControl = x.section(QLatin1Char('.'), 0, 0);
                            if (!xc.isValid() || seen == xc || xControl == h || xc.key == hc.key  // a shared key is reported above
                                || (single.contains(h) && single.contains(xControl))) {             // never down together
                                continue;
                            }
                            const auto as = cfg.hardware.lookup(seen);
                            if (as && as->name() != x && !told.contains(h + x)) {
                                told.insert(h + x);
                                QStringList mods = chordName(KeyChord{hc.mods, KEY_A}).split(QLatin1Char('+'));
                                mods.removeLast();
                                out << QStringLiteral("%1 %2: with \"input\": \"evdev\", %2 while %3 is held reads as %4 (the held key's %5 stays in the pad's report). Use raw input: control-surfaced set input auto (raw on firmware 2.0.2+)")
                                           .arg(where, x, h, as->name(), mods.join(QLatin1Char('+')));
                            }
                        }
                    }
                }
            }
        }
    }
    return out;
}

QStringList CheatsheetOptions::positions()
{
    return {QStringLiteral("center"), QStringLiteral("top"), QStringLiteral("bottom"), QStringLiteral("left"), QStringLiteral("right"),
            QStringLiteral("top-left"), QStringLiteral("top-right"), QStringLiteral("bottom-left"), QStringLiteral("bottom-right")};
}

QString EwwHook::anchorFor(const QString &position)
{
    static const QHash<QString, QString> anchors{
        {QStringLiteral("center"), QStringLiteral("center")},
        {QStringLiteral("top"), QStringLiteral("top center")},
        {QStringLiteral("bottom"), QStringLiteral("bottom center")},
        {QStringLiteral("left"), QStringLiteral("center left")},
        {QStringLiteral("right"), QStringLiteral("center right")},
        {QStringLiteral("top-left"), QStringLiteral("top left")},
        {QStringLiteral("top-right"), QStringLiteral("top right")},
        {QStringLiteral("bottom-left"), QStringLiteral("bottom left")},
        {QStringLiteral("bottom-right"), QStringLiteral("bottom right")},
    };
    return anchors.value(position, QStringLiteral("center"));
}

EwwHook EwwConfig::over(EwwHook h) const
{
    if (enabled) {
        h.enabled = *enabled;
    }
    if (variable) {
        h.variable = *variable;
    }
    if (window) {
        h.window = *window;
    }
    if (binary) {
        h.binary = *binary;
    }
    if (configDir) {
        h.configDir = *configDir;
    }
    return h;
}

} // namespace cs
