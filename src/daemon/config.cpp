// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"
#include "kdenlivecontract.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QStandardPaths>

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
        static const QRegularExpression slotRe(QStringLiteral("^(key([1-9]|1[0-5])|knob[1-3]\\.(turn|ccw|cw|press|shift\\.(turn|ccw|cw)))$"));
        if (!slotRe.match(slot).hasMatch()) {
            return fail(error, QStringLiteral("unknown control slot '%1' (key1..key15, knob1..knob3 with turn/ccw/cw/press/shift)").arg(slot));
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
            b.argv << a.toString();
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
    } else if (o.contains(QStringLiteral("request"))) {
        b.kind = Binding::Request;
        b.name = o.value(QStringLiteral("request")).toString();
        b.options = o.value(QStringLiteral("params")).toObject().toVariantMap();
    } else {
        if (error) {
            *error = QStringLiteral("binding object needs one of keys/action/control/command/cycle/request");
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
            if (l.when.isEmpty()) {
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

} // namespace cs
