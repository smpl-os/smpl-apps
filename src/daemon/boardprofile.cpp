// SPDX-License-Identifier: GPL-2.0-or-later
#include "boardprofile.h"

#include <QJsonArray>
#include <QRegularExpression>

namespace cs {

QJsonObject BoardProfile::toJson() const
{
    QJsonArray k, n;
    for (const BoardKey &key : keys) {
        k.append(QJsonObject{{QStringLiteral("control"), key.control},
                             {QStringLiteral("slot"), key.slot},
                             {QStringLiteral("row"), key.row},
                             {QStringLiteral("column"), key.column}});
    }
    for (const BoardKnob &knob : knobs) {
        n.append(QJsonObject{{QStringLiteral("control"), knob.control},
                             {QStringLiteral("slots"), QJsonObject{{QStringLiteral("ccw"), knob.ccw}, {QStringLiteral("press"), knob.press}, {QStringLiteral("cw"), knob.cw}}},
                             {QStringLiteral("row"), knob.row},
                             {QStringLiteral("column"), knob.column}});
    }
    return QJsonObject{{QStringLiteral("id"), id},
                       {QStringLiteral("name"), name},
                       {QStringLiteral("source"), source},
                       {QStringLiteral("rows"), rows},
                       {QStringLiteral("columns"), columns},
                       {QStringLiteral("slotCount"), slotCount()},
                       {QStringLiteral("keys"), k},
                       {QStringLiteral("knobs"), n}};
}

BoardProfile gridProfile(const QString &id, const QString &name, int keys, int knobs, int columns)
{
    BoardProfile p;
    p.id = id;
    p.name = name;
    p.source = QStringLiteral("template");
    columns = qMax(1, columns);
    const int keyRows = keys > 0 ? (keys + columns - 1) / columns : 0;
    for (int i = 0; i < keys; ++i) {
        p.keys << BoardKey{QStringLiteral("key%1").arg(i + 1), i, i / columns, i % columns};
    }
    for (int i = 0; i < knobs; ++i) {
        const int base = keys + 3 * i;
        p.knobs << BoardKnob{QStringLiteral("knob%1").arg(i + 1), base, base + 1, base + 2, i, keys > 0 ? columns : 0};
    }
    p.rows = qMax(keyRows, knobs);
    p.columns = (keys > 0 ? columns : 0) + (knobs > 0 ? 1 : 0);
    return p;
}

QList<BoardProfile> builtinBoardProfiles()
{
    // Measured with discovery.c on the user's pad (docs/FIRMWARE-PLAN.md §7.1),
    // held with the knobs on the right.
    BoardProfile measured = gridProfile(QStringLiteral("sy181-15k3e"), QStringLiteral("CH552G + TM1650, 15 keys, 3 knobs (SY181 style)"), 15, 3, 5);
    measured.source = QStringLiteral("measured");
    return {
        measured,
        gridProfile(QStringLiteral("generic-3k1e"), QStringLiteral("3 keys, 1 knob"), 3, 1, 3),
        gridProfile(QStringLiteral("generic-3k"), QStringLiteral("3 keys"), 3, 0, 3),
        gridProfile(QStringLiteral("generic-6k1e"), QStringLiteral("6 keys, 1 knob"), 6, 1, 3),
        gridProfile(QStringLiteral("generic-10k"), QStringLiteral("10 keys"), 10, 0, 5),
        gridProfile(QStringLiteral("generic-12k2e"), QStringLiteral("12 keys, 2 knobs"), 12, 2, 4),
        gridProfile(QStringLiteral("generic-12k3e"), QStringLiteral("12 keys, 3 knobs"), 12, 3, 4),
        gridProfile(QStringLiteral("generic-16k3e"), QStringLiteral("16 keys, 3 knobs"), 16, 3, 4),
    };
}

BoardProfile profileForControls(const QStringList &controls, const QString &source)
{
    static const QRegularExpression re(QStringLiteral("^(key|knob)(\\d+)"));
    int keys = 0, knobs = 0;
    for (const QString &c : controls) {
        const auto m = re.match(c);
        if (m.hasMatch()) {
            int &n = m.captured(1) == QLatin1String("key") ? keys : knobs;
            n = qMax(n, m.captured(2).toInt());
        }
    }
    BoardProfile p;
    if (keys == 15 && knobs == 3) {
        p = *builtinBoardProfile(QStringLiteral("sy181-15k3e"));
    } else {
        const int columns = keys >= 15 ? 5 : keys == 10 ? 5 : keys >= 7 ? 4 : qMax(1, qMin(keys, 3));
        p = gridProfile(QStringLiteral("grid-%1k%2e").arg(keys).arg(knobs), QStringLiteral("%1 keys, %2 knobs").arg(keys).arg(knobs), keys, knobs, columns);
    }
    p.source = source;
    return p;
}

std::optional<BoardProfile> builtinBoardProfile(const QString &id)
{
    for (const BoardProfile &p : builtinBoardProfiles()) {
        if (p.id == id) {
            return p;
        }
    }
    return std::nullopt;
}

} // namespace cs
