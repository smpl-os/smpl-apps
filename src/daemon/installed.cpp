// SPDX-License-Identifier: GPL-2.0-or-later
#include "installed.h"

#include <QDateTime>
#include <QFileInfo>
#include <QHash>
#include <QStandardPaths>

namespace cs {

namespace {
std::function<bool(const QString &)> &override()
{
    static std::function<bool(const QString &)> f;
    return f;
}

struct Entry {
    bool installed = false;
    qint64 checkedAt = 0;
};

QHash<QString, Entry> &cache()
{
    static QHash<QString, Entry> c;
    return c;
}

bool lookUp(const QString &name)
{
    if (name.startsWith(QLatin1Char('/'))) {
        const QFileInfo f(name);
        return f.isFile() && f.isExecutable();
    }
    if (!QStandardPaths::findExecutable(name).isEmpty()) {
        return true;
    }
    const QString id = name.endsWith(QLatin1String(".desktop")) ? name : name + QStringLiteral(".desktop");
    return !QStandardPaths::locate(QStandardPaths::ApplicationsLocation, id).isEmpty();
}
} // namespace

bool isInstalled(const QString &name)
{
    if (override()) {
        return override()(name);
    }
    const qint64 now = QDateTime::currentMSecsSinceEpoch();
    Entry &e = cache()[name];
    if (e.checkedAt == 0 || now - e.checkedAt > 30000) {
        e.installed = lookUp(name);
        e.checkedAt = now;
    }
    return e.installed;
}

bool bindingAvailable(const Binding &b)
{
    for (const QString &n : b.ifInstalled) {
        if (!isInstalled(n)) {
            return false;
        }
    }
    return true;
}

void setInstalledCheck(std::function<bool(const QString &)> check)
{
    override() = std::move(check);
    cache().clear();
}

} // namespace cs
