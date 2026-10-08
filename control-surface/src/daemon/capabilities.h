// SPDX-License-Identifier: GPL-2.0-or-later
// `control-surfaced list-capabilities`: asks a running Kdenlive (or the mock)
// what its ControlSurface1 interface offers right now, without a lease, and
// relates it to the config: which layers apply now and which configured
// controls, commands and actions this Kdenlive does not offer.
#pragma once

#include "config.h"

#include <QDBusConnection>
#include <QJsonObject>
#include <QStringList>
#include <QVariantMap>

namespace cs {

struct CapabilityReport {
    QString service;  // empty on a peer-to-peer connection
    enum class Status { Available, Absent, Incompatible, Error } status = Status::Error;
    QString detail;  // D-Bus error or refusal for the non-available states
    QVariantMap capabilities;
    QVariantList actions;  // {id, text, enabled, checkable, checked, shortcut}
    QVariantMap context;
};

// org.kde.kdenlive-<pid> names on the bus, sorted.
QStringList discoverKdenliveServices(const QDBusConnection &conn);
// Capabilities, ListActions and GetContext (no Subscribe, so no lease).
CapabilityReport queryKdenlive(const QDBusConnection &conn, const QString &service, int timeoutMs = 3000);

struct ConfigFindings {
    QStringList layersNow;  // Kdenlive-profile layers whose "when" matches now (current modes)
    QStringList notOffered; // "action keyframe_add (profile kdenlive layer effect-parameter key10)"
};
// modeValues: current mode values (name -> value); missing modes use their first value.
ConfigFindings checkAgainstConfig(const CapabilityReport &r, const Config &cfg, const QVariantMap &modeValues = {});

QString formatReport(const CapabilityReport &r, const Config *cfg);
QJsonObject reportJson(const CapabilityReport &r, const Config *cfg);

// Dotted paths of scalar context values ("timeline.track.audio" -> false),
// lists as their length: what layer "when" conditions can test.
QVariantMap flattenContext(const QVariantMap &ctx, int maxDepth = 4);

} // namespace cs
