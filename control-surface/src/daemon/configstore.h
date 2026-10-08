// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "config.h"

#include <QByteArray>
#include <QJsonObject>
#include <QString>
#include <QStringList>
#include <optional>

namespace cs {

// Reads, validates and safely replaces the config file for the settings API.
// A write never happens unless the text validates and the file still has the
// content the caller last read; the previous file is kept as <path>.bak.
class ConfigStore
{
public:
    explicit ConfigStore(const QString &path) : m_path(path) {}
    QString path() const { return m_path; }

    static constexpr int kMaxBytes = 1 << 20;
    static QString hashOf(const QByteArray &text);  // sha256, lower-case hex

    struct Snapshot {
        bool exists = false;
        QByteArray text;
        QString hash;  // empty when the file does not exist
    };
    Snapshot read() const;

    struct Validation {
        bool ok = false;
        QStringList errors;
        QStringList warnings;
        std::optional<Config> config;
        QJsonObject toJson() const;  // ok, errors, warnings, profiles, hardwareSource
    };
    Validation validate(const QByteArray &text) const;

    struct WriteResult {
        bool ok = false;
        QString error;       // code: invalid | hash-mismatch | too-large | io
        QString message;
        QString hash;        // of the file now on disk
        QString backup;      // path of the previous version, if there was one
        Validation validation;
        QJsonObject toJson() const;
    };
    // expectedHash: the hash from read(); empty means "the file must not exist yet".
    WriteResult write(const QByteArray &text, const QString &expectedHash) const;

private:
    QString m_path;
};

} // namespace cs
