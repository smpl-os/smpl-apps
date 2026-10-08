// SPDX-License-Identifier: GPL-2.0-or-later
// A stand-in eww for tests: an executable named "eww" that records each call
// (one line, arguments separated by 0x1f) in $EWW_MOCK_LOG and exits 0.
// Flag files next to it change that: "fail-update" makes `update` fail the
// way eww does without a daemon; "slow" makes every call take 300 ms.
#pragma once

#include <QDir>
#include <QFile>
#include <QStringList>

namespace ewwmock {

inline bool install(const QString &dir)
{
    const QString path = dir + QStringLiteral("/eww");
    QFile f(path);
    if (!f.open(QIODevice::WriteOnly)) {
        return false;
    }
    f.write(R"(#!/bin/sh
here=$(dirname "$0")
log="${EWW_MOCK_LOG:?}"
line=""
sep=""
verb=""
for a in "$@"; do
    line="$line$sep$a"
    sep=$(printf '\037')
    case "$a" in update|open|close) [ -z "$verb" ] && verb=$a ;; esac
done
printf '%s\n' "$line" >> "$log"
[ -e "$here/slow" ] && sleep 0.3
if [ "$verb" = update ] && [ -e "$here/fail-update" ]; then
    echo "2026-10-07T23:01:13.035Z ERROR eww::error_handling_ctx > Failed to connect to daemon" >&2
    exit 1
fi
exit 0
)");
    f.close();
    return f.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
}

inline QList<QStringList> calls(const QString &log)
{
    QFile f(log);
    QList<QStringList> out;
    if (!f.open(QIODevice::ReadOnly)) {
        return out;
    }
    for (const QByteArray &line : f.readAll().split('\n')) {
        if (!line.isEmpty()) {
            out << QString::fromUtf8(line).split(QChar(0x1f));
        }
    }
    return out;
}

// The verb and its operand, e.g. "update visible=false", "open pad-cheatsheet top left",
// "close pad-cheatsheet"; options before the verb (--config DIR) are dropped.
inline QString summary(const QStringList &call, QString *configDir = nullptr)
{
    int i = 0;
    if (call.value(0) == QLatin1String("--config")) {
        if (configDir) {
            *configDir = call.value(1);
        }
        i = 2;
    }
    const QString verb = call.value(i);
    if (verb == QLatin1String("update")) {
        const QString arg = call.value(i + 1);
        const int eq = arg.indexOf(QLatin1Char('='));
        const bool visible = arg.mid(eq + 1).contains(QLatin1String("\"visible\":true"));
        return QStringLiteral("update %1 visible=%2").arg(arg.left(eq), visible ? QStringLiteral("true") : QStringLiteral("false"));
    }
    if (verb == QLatin1String("open")) {
        return QStringLiteral("open %1%2").arg(call.value(i + 1), call.value(i + 2) == QLatin1String("--anchor") ? QStringLiteral(" @") + call.value(i + 3) : QString());
    }
    return call.mid(i).join(QLatin1Char(' '));
}

inline QStringList summaries(const QString &log, QString *configDir = nullptr)
{
    QStringList out;
    for (const QStringList &c : calls(log)) {
        out << summary(c, configDir);
    }
    return out;
}

} // namespace ewwmock
