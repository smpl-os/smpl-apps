// SPDX-License-Identifier: GPL-2.0-or-later
// Hot reload: watches the config file (and the directory, so editors that
// save by rename and files created later are seen), debounces bursts of
// changes and re-parses with the same parser as startup and check-config. A
// config that fails to load never replaces the running one; it is reported
// only if it still fails a moment later (setRetryMs) with no new change in
// between, so a writer that writes in place, in pieces, is not mistaken for
// a broken file.
#pragma once

#include "config.h"

#include <QFileSystemWatcher>
#include <QObject>
#include <QTimer>

namespace cs {

class ConfigWatcher : public QObject
{
    Q_OBJECT
public:
    explicit ConfigWatcher(const QString &path, QObject *parent = nullptr);
    // Other files whose changes reload too (the learned hardware map).
    void setExtraFiles(const QStringList &paths);
    void setDebounceMs(int ms) { m_debounce.setInterval(ms); }
    void setRetryMs(int ms) { m_retry.setInterval(ms); }
    void start();
    QString path() const { return m_path; }

Q_SIGNALS:
    void reloaded(const cs::Config &cfg);
    void failed(const QString &error);

private:
    void arm();
    void reload(bool retry);
    QByteArray fingerprint() const;
    QString m_path;
    QStringList m_extra;
    QFileSystemWatcher m_watcher;
    QTimer m_debounce;
    QTimer m_retry;
    QByteArray m_last;
};

} // namespace cs
