// SPDX-License-Identifier: GPL-2.0-or-later
#include "capabilities.h"
#include "kdenlivecontract.h"
#include "kdenlivedbusclient.h"

#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QSet>
#include <functional>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRegularExpression>

namespace cs {

namespace {

QString statusName(CapabilityReport::Status s)
{
    switch (s) {
    case CapabilityReport::Status::Available:
        return QStringLiteral("available");
    case CapabilityReport::Status::Absent:
        return QStringLiteral("absent");
    case CapabilityReport::Status::Incompatible:
        return QStringLiteral("incompatible");
    case CapabilityReport::Status::Error:
        break;
    }
    return QStringLiteral("error");
}

bool isAbsentError(const QString &name)
{
    // The interface is off (no object), or Kdenlive is not there at all.
    return name == QLatin1String("org.freedesktop.DBus.Error.UnknownObject") || name == QLatin1String("org.freedesktop.DBus.Error.UnknownInterface")
        || name == QLatin1String("org.freedesktop.DBus.Error.UnknownMethod") || name == QLatin1String("org.freedesktop.DBus.Error.ServiceUnknown")
        || name == QLatin1String("org.freedesktop.DBus.Error.NameHasNoOwner");
}

QString compactJson(const QVariant &v)
{
    const QJsonValue j = QJsonValue::fromVariant(v);
    if (j.isObject()) {
        return QString::fromUtf8(QJsonDocument(j.toObject()).toJson(QJsonDocument::Compact));
    }
    if (j.isArray()) {
        return QString::fromUtf8(QJsonDocument(j.toArray()).toJson(QJsonDocument::Compact));
    }
    if (j.isString()) {
        return j.toString();
    }
    if (j.isBool()) {
        return j.toBool() ? QStringLiteral("true") : QStringLiteral("false");
    }
    return j.isDouble() ? QString::number(j.toDouble()) : QStringLiteral("null");
}

const QStringList kHandles{QStringLiteral("param.target"),
                           QStringLiteral("colorWheel.target"),
                           QStringLiteral("hoveredColorWheel.target"),
                           QStringLiteral("timeline.track.target"),
                           QStringLiteral("timeline.track.gain.target"),
                           QStringLiteral("timeline.clipGain.target"),
                           QStringLiteral("timeline.trim.target")};

} // namespace

QStringList discoverKdenliveServices(const QDBusConnection &conn)
{
    QStringList out;
    if (!conn.interface()) {
        return out;
    }
    static const QRegularExpression re(QStringLiteral("^org\\.kde\\.kdenlive-\\d+$"));
    const QStringList names = conn.interface()->registeredServiceNames().value();
    for (const QString &n : names) {
        if (re.match(n).hasMatch()) {
            out << n;
        }
    }
    out.sort();
    return out;
}

CapabilityReport queryKdenlive(const QDBusConnection &conn, const QString &service, int timeoutMs)
{
    CapabilityReport r;
    r.service = service;
    auto call = [&](const QString &method, Envelope *out) -> bool {
        auto msg = QDBusMessage::createMethodCall(service, contract::kPath, contract::kInterface, method);
        // BlockWithGui keeps the event loop running (an in-process peer can answer).
        const QDBusMessage reply = QDBusConnection(conn).call(msg, QDBus::BlockWithGui, timeoutMs);
        if (reply.type() == QDBusMessage::ErrorMessage) {
            r.status = isAbsentError(reply.errorName()) ? CapabilityReport::Status::Absent : CapabilityReport::Status::Error;
            r.detail = QStringLiteral("%1: %2 (%3)").arg(method, reply.errorName(), reply.errorMessage());
            return false;
        }
        *out = Envelope::parse(reply.arguments().value(0));
        if (!out->valid || !out->ok) {
            r.status = CapabilityReport::Status::Error;
            r.detail = QStringLiteral("%1 refused: %2 %3").arg(method, out->code, out->message);
            return false;
        }
        return true;
    };
    Envelope e;
    if (!call(QStringLiteral("Capabilities"), &e)) {
        return r;
    }
    r.capabilities = e.result;
    const uint version = e.result.value(QStringLiteral("version")).toUInt();
    const uint revision = e.result.value(QStringLiteral("revision")).toUInt();
    if (version != contract::kVersion || revision < contract::kRevision) {
        r.status = CapabilityReport::Status::Incompatible;
        r.detail = QStringLiteral("version %1 revision %2; this daemon needs version %3 revision >= %4").arg(version).arg(revision).arg(contract::kVersion).arg(contract::kRevision);
        return r;
    }
    if (!call(QStringLiteral("ListActions"), &e)) {
        return r;
    }
    r.actions = e.result.value(QStringLiteral("actions")).toList();
    if (!call(QStringLiteral("GetContext"), &e)) {
        return r;
    }
    r.context = e.result;
    r.status = CapabilityReport::Status::Available;
    return r;
}

QVariantMap flattenContext(const QVariantMap &ctx, int maxDepth)
{
    QVariantMap out;
    std::function<void(const QString &, const QVariant &, int)> walk = [&](const QString &prefix, const QVariant &v, int depth) {
        if (v.metaType() == QMetaType::fromType<QVariantMap>() && depth < maxDepth) {
            const QVariantMap m = v.toMap();
            for (auto it = m.cbegin(); it != m.cend(); ++it) {
                walk(prefix.isEmpty() ? it.key() : prefix + QLatin1Char('.') + it.key(), it.value(), depth + 1);
            }
        } else if (v.canConvert<QVariantList>() && v.metaType() != QMetaType::fromType<QString>()) {
            out.insert(prefix, QStringLiteral("[%1 items]").arg(v.toList().size()));
        } else {
            out.insert(prefix, v);
        }
    };
    walk(QString(), ctx, 0);
    return out;
}

ConfigFindings checkAgainstConfig(const CapabilityReport &r, const Config &cfg, const QVariantMap &modeValues)
{
    ConfigFindings f;
    const QStringList controls = r.capabilities.value(QStringLiteral("controls")).toStringList();
    const QStringList commands = r.capabilities.value(QStringLiteral("commands")).toStringList();
    QSet<QString> actions;
    for (const auto &a : r.actions) {
        actions.insert(a.toMap().value(QStringLiteral("id")).toString());
    }
    for (const Profile &p : cfg.profiles) {
        if (!p.kdenlive) {
            continue;
        }
        QVariantMap modes;
        for (auto it = p.modes.cbegin(); it != p.modes.cend(); ++it) {
            modes.insert(it.key(), modeValues.value(it.key(), it.value().value(0)));
        }
        QVariantMap ctx = r.context;
        ctx.insert(QStringLiteral("$mode"), modes);
        auto check = [&](const QString &where, const Binding &b) {
            QStringList names{b.name};
            if (b.kind == Binding::Control && b.name.startsWith(QLatin1Char('$'))) {
                names = p.modes.value(b.name.mid(1));  // every control the mode can select
            }
            for (const QString &n : std::as_const(names)) {
                QString kind;
                if (b.kind == Binding::Control && !controls.contains(n)) {
                    kind = QStringLiteral("control");
                } else if (b.kind == Binding::Request && !commands.contains(n)) {
                    kind = QStringLiteral("command");
                } else if (b.kind == Binding::Action && !actions.contains(n)) {
                    kind = QStringLiteral("action");
                }
                if (!kind.isEmpty()) {
                    f.notOffered << QStringLiteral("%1 %2 (%3)").arg(kind, n, where);
                }
            }
        };
        for (const Layer &l : p.layers) {
            if (conditionMatches(l.when, ctx)) {
                f.layersNow << QStringLiteral("%1/%2").arg(p.name, l.name);
            }
            for (auto it = l.bindings.cbegin(); it != l.bindings.cend(); ++it) {
                check(QStringLiteral("profile %1 layer %2 %3").arg(p.name, l.name, it.key()), it.value());
            }
        }
        for (auto it = p.bindings.cbegin(); it != p.bindings.cend(); ++it) {
            check(QStringLiteral("profile %1 %2").arg(p.name, it.key()), it.value());
        }
    }
    f.notOffered.sort();
    return f;
}

QString formatReport(const CapabilityReport &r, const Config *cfg)
{
    QStringList out;
    const QString who = r.service.isEmpty() ? QStringLiteral("Kdenlive (peer)") : QStringLiteral("Kdenlive %1").arg(r.service);
    switch (r.status) {
    case CapabilityReport::Status::Absent:
        out << QStringLiteral("%1: control interface not enabled (%2).").arg(who, r.detail)
            << QStringLiteral("  Enable \"Control surface interface\" in Kdenlive's settings (enableControlSurfaceInterface, off by default).");
        return out.join(QLatin1Char('\n'));
    case CapabilityReport::Status::Incompatible:
        out << QStringLiteral("%1: incompatible %2: %3").arg(who, contract::kInterface, r.detail);
        return out.join(QLatin1Char('\n'));
    case CapabilityReport::Status::Error:
        out << QStringLiteral("%1: error: %2").arg(who, r.detail);
        return out.join(QLatin1Char('\n'));
    case CapabilityReport::Status::Available:
        break;
    }
    const QVariantMap &c = r.capabilities;
    out << QStringLiteral("%1: %2 version %3 revision %4%5")
               .arg(who, contract::kInterface)
               .arg(c.value(QStringLiteral("version")).toUInt())
               .arg(c.value(QStringLiteral("revision")).toUInt())
               .arg(c.contains(QStringLiteral("implementation")) ? QStringLiteral(" (%1)").arg(c.value(QStringLiteral("implementation")).toString()) : QString());
    const QStringList controls = c.value(QStringLiteral("controls")).toStringList();
    const QStringList commands = c.value(QStringLiteral("commands")).toStringList();
    out << QStringLiteral("controls (%1): %2").arg(controls.size()).arg(controls.join(QLatin1Char(' ')));
    out << QStringLiteral("commands (%1): %2").arg(commands.size()).arg(commands.join(QLatin1Char(' ')));
    if (c.contains(QStringLiteral("trimModes"))) {
        out << QStringLiteral("trim modes: %1").arg(c.value(QStringLiteral("trimModes")).toStringList().join(QLatin1Char(' ')));
    }
    QStringList limits;
    const QVariantMap lm = c.value(QStringLiteral("limits")).toMap();
    for (auto it = lm.cbegin(); it != lm.cend(); ++it) {
        limits << QStringLiteral("%1=%2").arg(it.key(), compactJson(it.value()));
    }
    out << QStringLiteral("limits: %1").arg(limits.join(QLatin1Char(' ')));
    out << QStringLiteral("actions (%1):").arg(r.actions.size());
    for (const auto &a : r.actions) {
        const QVariantMap m = a.toMap();
        const QString shortcut = m.value(QStringLiteral("shortcut")).toString();
        out << QStringLiteral("  %1  \"%2\"%3%4")
                   .arg(m.value(QStringLiteral("id")).toString(), m.value(QStringLiteral("text")).toString().remove(QLatin1Char('&')))
                   .arg(shortcut.isEmpty() ? QString() : QStringLiteral(" [%1]").arg(shortcut),
                        m.value(QStringLiteral("enabled"), true).toBool() ? QString() : QStringLiteral(" (disabled now)"));
    }
    const QVariantMap flat = flattenContext(r.context);
    QStringList now;
    for (const QString &k : {QStringLiteral("focus"), QStringLiteral("activeMonitor"), QStringLiteral("tool"), QStringLiteral("playing"), QStringLiteral("position"),
                             QStringLiteral("epoch")}) {
        if (flat.contains(k)) {
            now << QStringLiteral("%1=%2").arg(k, compactJson(flat.value(k)));
        }
    }
    out << QStringLiteral("context now: %1").arg(now.join(QLatin1Char(' ')));
    QStringList handles;
    for (const QString &h : kHandles) {
        if (flat.contains(h)) {
            handles << h;
        }
    }
    if (r.context.contains(QStringLiteral("colorWheels"))) {
        handles << QStringLiteral("colorWheels[%1]").arg(r.context.value(QStringLiteral("colorWheels")).toList().size());
    }
    out << QStringLiteral("  editing handles now: %1").arg(handles.isEmpty() ? QStringLiteral("(none)") : handles.join(QLatin1Char(' ')));
    out << QStringLiteral("  focus values for \"when\": timeline clipMonitor projectMonitor effectStack other");
    out << QStringLiteral("  context paths for \"when\" (current values):");
    for (auto it = flat.cbegin(); it != flat.cend(); ++it) {
        out << QStringLiteral("    %1 = %2").arg(it.key(), compactJson(it.value()));
    }
    if (cfg) {
        const ConfigFindings f = checkAgainstConfig(r, *cfg);
        out << QStringLiteral("config: layers that apply now (modes at their first value): %1").arg(f.layersNow.isEmpty() ? QStringLiteral("(none; profile bindings apply)") : f.layersNow.join(QLatin1Char(' ')));
        if (f.notOffered.isEmpty()) {
            out << QStringLiteral("config: everything bound is offered by this Kdenlive");
        } else {
            out << QStringLiteral("config: bound but not offered by this Kdenlive (these inputs do nothing here) (%1):").arg(f.notOffered.size());
            for (const QString &n : f.notOffered) {
                out << QStringLiteral("  %1").arg(n);
            }
        }
    }
    return out.join(QLatin1Char('\n'));
}

QJsonObject reportJson(const CapabilityReport &r, const Config *cfg)
{
    QJsonObject o{{QStringLiteral("service"), r.service}, {QStringLiteral("status"), statusName(r.status)}};
    if (!r.detail.isEmpty()) {
        o.insert(QStringLiteral("detail"), r.detail);
    }
    if (r.status == CapabilityReport::Status::Available || r.status == CapabilityReport::Status::Incompatible) {
        o.insert(QStringLiteral("capabilities"), QJsonObject::fromVariantMap(r.capabilities));
    }
    if (r.status == CapabilityReport::Status::Available) {
        o.insert(QStringLiteral("actions"), QJsonArray::fromVariantList(r.actions));
        o.insert(QStringLiteral("context"), QJsonObject::fromVariantMap(r.context));
        o.insert(QStringLiteral("contextPaths"), QJsonObject::fromVariantMap(flattenContext(r.context)));
        if (cfg) {
            const ConfigFindings f = checkAgainstConfig(r, *cfg);
            o.insert(QStringLiteral("layersNow"), QJsonArray::fromStringList(f.layersNow));
            o.insert(QStringLiteral("notOffered"), QJsonArray::fromStringList(f.notOffered));
        }
    }
    return o;
}

} // namespace cs
