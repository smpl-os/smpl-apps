// SPDX-License-Identifier: GPL-2.0-or-later
#include "configstore.h"

#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QSaveFile>

namespace cs {

QString ConfigStore::hashOf(const QByteArray &text)
{
    return QString::fromLatin1(QCryptographicHash::hash(text, QCryptographicHash::Sha256).toHex());
}

ConfigStore::Snapshot ConfigStore::read() const
{
    Snapshot s;
    QFile f(m_path);
    if (!f.exists() || !f.open(QIODevice::ReadOnly)) {
        return s;
    }
    s.exists = true;
    s.text = f.readAll();
    s.hash = hashOf(s.text);
    return s;
}

QJsonObject ConfigStore::Validation::toJson() const
{
    QJsonArray profiles;
    if (config) {
        for (const Profile &p : config->profiles) {
            profiles.append(QJsonObject{{QStringLiteral("name"), p.name},
                                        {QStringLiteral("layers"), int(p.layers.size())},
                                        {QStringLiteral("bindings"), int(p.bindings.size())},
                                        {QStringLiteral("kdenlive"), p.kdenlive},
                                        {QStringLiteral("keyFallback"), p.keyFallback}});
        }
    }
    return QJsonObject{{QStringLiteral("ok"), ok},
                       {QStringLiteral("errors"), QJsonArray::fromStringList(errors)},
                       {QStringLiteral("warnings"), QJsonArray::fromStringList(warnings)},
                       {QStringLiteral("profiles"), profiles},
                       {QStringLiteral("hardwareSource"), config ? config->hardwareSource : QString()}};
}

ConfigStore::Validation ConfigStore::validate(const QByteArray &text) const
{
    Validation v;
    if (text.size() > kMaxBytes) {
        v.errors << QStringLiteral("config is larger than %1 bytes").arg(kMaxBytes);
        return v;
    }
    QString err;
    auto cfg = parseConfig(text, QFileInfo(m_path).absolutePath(), &err);
    if (!cfg) {
        v.errors << (err.isEmpty() ? QStringLiteral("invalid config") : err);
        return v;
    }
    v.ok = true;
    v.warnings = cfg->warnings;
    v.config = std::move(cfg);
    return v;
}

QJsonObject ConfigStore::WriteResult::toJson() const
{
    QJsonObject o{{QStringLiteral("ok"), ok}, {QStringLiteral("hash"), hash}, {QStringLiteral("backup"), backup}};
    if (!ok) {
        o.insert(QStringLiteral("error"), QJsonObject{{QStringLiteral("code"), error}, {QStringLiteral("message"), message}});
    }
    o.insert(QStringLiteral("errors"), QJsonArray::fromStringList(validation.errors));
    o.insert(QStringLiteral("warnings"), QJsonArray::fromStringList(validation.warnings));
    return o;
}

ConfigStore::WriteResult ConfigStore::write(const QByteArray &text, const QString &expectedHash) const
{
    WriteResult r;
    const Snapshot now = read();
    r.hash = now.hash;
    if (text.size() > kMaxBytes) {
        r.error = QStringLiteral("too-large");
        r.message = QStringLiteral("config is larger than %1 bytes").arg(kMaxBytes);
        return r;
    }
    r.validation = validate(text);
    if (!r.validation.ok) {
        r.error = QStringLiteral("invalid");
        r.message = r.validation.errors.join(QStringLiteral("; "));
        return r;
    }
    if (now.hash != expectedHash) {
        r.error = QStringLiteral("hash-mismatch");
        r.message = now.exists ? QStringLiteral("the config file changed since it was read; read it again and merge")
                               : QStringLiteral("the config file does not exist; pass an empty expected hash to create it");
        return r;
    }
    if (now.exists && now.text == text) {
        r.ok = true;  // nothing to do
        return r;
    }
    QDir().mkpath(QFileInfo(m_path).absolutePath());
    if (now.exists) {
        // Keep the version being replaced. QSaveFile makes the copy atomic too.
        const QString bak = m_path + QStringLiteral(".bak");
        QSaveFile b(bak);
        if (!b.open(QIODevice::WriteOnly) || b.write(now.text) != now.text.size() || !b.commit()) {
            r.error = QStringLiteral("io");
            r.message = QStringLiteral("cannot write backup %1").arg(bak);
            return r;
        }
        r.backup = bak;
    }
    QSaveFile f(m_path);
    if (!f.open(QIODevice::WriteOnly) || f.write(text) != text.size() || !f.commit()) {
        r.error = QStringLiteral("io");
        r.message = QStringLiteral("cannot write %1").arg(m_path);
        return r;
    }
    r.ok = true;
    r.hash = hashOf(text);
    return r;
}

} // namespace cs
