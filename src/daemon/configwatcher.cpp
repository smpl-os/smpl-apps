// SPDX-License-Identifier: GPL-2.0-or-later
#include "configwatcher.h"

#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>

namespace cs {

ConfigWatcher::ConfigWatcher(const QString &path, QObject *parent)
    : QObject(parent)
    , m_path(QFileInfo(path).absoluteFilePath())
{
    m_debounce.setSingleShot(true);
    m_debounce.setInterval(300);
    m_retry.setSingleShot(true);
    m_retry.setInterval(1000);
    connect(&m_debounce, &QTimer::timeout, this, [this] { reload(false); });
    connect(&m_retry, &QTimer::timeout, this, [this] { reload(true); });
    connect(&m_watcher, &QFileSystemWatcher::fileChanged, this, [this] {
        m_retry.stop();  // still being written: the next reload decides
        arm();  // a replaced file drops out of the watch list: add it again
        m_debounce.start();
    });
    connect(&m_watcher, &QFileSystemWatcher::directoryChanged, this, [this] {
        m_retry.stop();
        arm();
        m_debounce.start();
    });
}

void ConfigWatcher::setExtraFiles(const QStringList &paths)
{
    m_extra.clear();
    for (const QString &p : paths) {
        m_extra << QFileInfo(p).absoluteFilePath();
    }
}

void ConfigWatcher::start()
{
    m_last = fingerprint();
    arm();
}

void ConfigWatcher::arm()
{
    QStringList files{m_path};
    files << m_extra;
    QStringList wanted;
    for (const QString &f : std::as_const(files)) {
        const QString dir = QFileInfo(f).absolutePath();
        if (QFileInfo::exists(dir) && !wanted.contains(dir)) {
            wanted << dir;
        }
        if (QFileInfo::exists(f)) {
            wanted << f;
        }
    }
    const QStringList watched = m_watcher.files() + m_watcher.directories();
    QStringList add;
    for (const QString &w : std::as_const(wanted)) {
        if (!watched.contains(w)) {
            add << w;
        }
    }
    if (!add.isEmpty()) {
        m_watcher.addPaths(add);
    }
}

QByteArray ConfigWatcher::fingerprint() const
{
    QCryptographicHash h(QCryptographicHash::Sha256);
    for (const QString &f : QStringList{m_path} + m_extra) {
        QFile file(f);
        h.addData(f.toUtf8());
        if (file.open(QIODevice::ReadOnly)) {
            h.addData(QByteArrayLiteral("\x01"));
            h.addData(file.readAll());
        } else {
            h.addData(QByteArrayLiteral("\x00"));
        }
    }
    return h.result();
}

void ConfigWatcher::reload(bool retry)
{
    const QByteArray now = fingerprint();
    if (now == m_last && !retry) {
        return;  // touched or re-saved without changes
    }
    m_last = now;
    QString err;
    std::optional<Config> cfg;
    if (!QFileInfo::exists(m_path)) {
        err = QStringLiteral("%1 was removed; keeping the running config").arg(m_path);
    } else {
        cfg = loadConfig(m_path, &err);  // the parser startup and check-config use
        if (!cfg) {
            err = QStringLiteral("%1: %2; keeping the running config").arg(m_path, err);
        }
    }
    if (cfg) {
        m_retry.stop();
        Q_EMIT reloaded(*cfg);
    } else if (!retry) {
        m_retry.start();  // maybe half written (an editor writing in place): look again
    } else {
        Q_EMIT failed(err);
    }
}

} // namespace cs
