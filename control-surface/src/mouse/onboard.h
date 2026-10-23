// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QJsonObject>
#include <QMap>
#include <functional>
#include <optional>

namespace cs::mouse::onboard {

enum class ReplyState { Ready, Busy, Unrelated, Error };
struct Reply {
    ReplyState state = ReplyState::Error;
    QByteArray arguments;
    QString error;
};

QByteArray readReport(quint16 command, const QByteArray &arguments, int responseSize);
Reply decodeReply(const QByteArray &report, quint16 command, int responseSize);
QStringList assignmentAddresses();

using Read = std::function<std::optional<QByteArray>(quint16, const QByteArray &, int, QString *)>;
std::optional<QJsonObject> backup(const Read &read, QString *error);
std::optional<QJsonObject> probe(const Read &read, QString *error);
std::optional<QJsonObject> dpiPlan(const QJsonObject &saved, const QJsonObject &current, QString *error);
QByteArray dpiWriteReport(int button, const QByteArray &function);
enum class DpiOperation { Apply, Restore };
using WriteDpi = std::function<bool(int, const QByteArray &, QString *)>;
using Snapshot = std::function<std::optional<QJsonObject>(QString *)>;
std::optional<QJsonObject> changeDpi(const QJsonObject &saved, const QJsonObject &before, DpiOperation operation,
                                   const WriteDpi &write, const Snapshot &snapshot, QString *error);

// Mutation requires its exact confirmation flag and the verified backup's SHA-256.
int run(const QStringList &arguments);

} // namespace cs::mouse::onboard
