// SPDX-License-Identifier: GPL-2.0-or-later
#include "ch552proto.h"
#include "hidrawdev.h"
#include "scheme.h"

#include <QSet>
#include <QTest>

using namespace ch552;

namespace {
std::vector<std::uint8_t> bytes(std::initializer_list<int> l)
{
    std::vector<std::uint8_t> v;
    for (int b : l) {
        v.push_back(std::uint8_t(b));
    }
    return v;
}
bool starts(const Frame &f, std::initializer_list<int> l)
{
    std::size_t i = 0;
    for (int b : l) {
        if (f[i++] != b) {
            return false;
        }
    }
    for (; i < f.size(); ++i) {
        if (f[i] != 0) {
            return false;
        }
    }
    return true;
}
} // namespace

class TestProto : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void rid0BindingMatchesVendorApp()
    {
        // MINI KeyBoard.exe Download_Click with ReportID 0: type byte without layer, header then step, then AA AA.
        const auto f = keyBinding(Generation::Rid0, 7, {Chord{mod::LShift, 0x6a}});
        QCOMPARE(f.size(), std::size_t(3));
        QVERIFY(starts(f[0], {7, 0x01, 1, 0, 0x02, 0}));
        QVERIFY(starts(f[1], {7, 0x01, 1, 1, 0x02, 0x6a}));
        QVERIFY(starts(f[2], {0xAA, 0xAA}));
        QCOMPARE(reportIdFor(Generation::Rid0), std::uint8_t(0));
    }
    void rid3BindingHasLayerSelectAndNibble()
    {
        const auto f = keyBinding(Generation::Rid3, 13, {Chord{0, 0x68}}, 2);
        QCOMPARE(f.size(), std::size_t(4));
        QVERIFY(starts(f[0], {0xA1, 2}));
        QVERIFY(starts(f[1], {13, 0x21, 1, 0, 0, 0}));
        QVERIFY(starts(f[2], {13, 0x21, 1, 1, 0, 0x68}));
        QCOMPARE(reportIdFor(Generation::Rid3), std::uint8_t(3));
    }
    void multiStepMacro()
    {
        const auto f = keyBinding(Generation::Rid0, 1, {Chord{mod::LCtrl, 0x0e}, Chord{0, 0x1b}});
        QCOMPARE(f.size(), std::size_t(4));
        QVERIFY(starts(f[0], {1, 1, 2, 0, mod::LCtrl, 0}));
        QVERIFY(starts(f[2], {1, 1, 2, 2, 0, 0x1b}));
    }
    void emptyKeyIsHeaderOnly()
    {
        const auto f = emptyKey(Generation::Rid0, 24);
        QCOMPARE(f.size(), std::size_t(2));
        QVERIFY(starts(f[0], {24, 1, 1, 0, 0, 0}));
        QVERIFY(starts(f[1], {0xAA, 0xAA}));
    }
    void rejectsBadInput()
    {
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument, keyBinding(Generation::Rid0, 0, {Chord{0, 4}}));
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument, keyBinding(Generation::Rid0, 25, {Chord{0, 4}}));
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument, keyBinding(Generation::Rid0, 1, {}));
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument, keyBinding(Generation::Rid0, 1, {Chord{0, 0x92}}));
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument,
                                 keyBinding(Generation::Rid0, 1, {Chord{0, 4}, Chord{0, 4}, Chord{0, 4}, Chord{0, 4}, Chord{0, 4}, Chord{0, 4}}));
    }
    void allowList()
    {
        QVERIFY(isAllowedFrame(pingFrame()));
        QVERIFY(isAllowedFrame(commitFrame()));
        for (int op : {0xEF, 0x5A, 0xFC, 0xFE, 0xB0, 0xFD}) {
            Frame f{};
            f[0] = std::uint8_t(op);
            QVERIFY(!isAllowedFrame(f));
        }
        Frame bad{};
        bad[0] = 3;
        bad[1] = 0x02;  // media type: never produced by the flasher
        QVERIFY(!isAllowedFrame(bad));
        Frame dirtyPing{};
        dirtyPing[5] = 1;
        QVERIFY(!isAllowedFrame(dirtyPing));
        for (const auto &sc : defaultScheme()) {
            for (const auto &f : keyBinding(Generation::Rid0, sc.slot, {sc.chord})) {
                QVERIFY(isAllowedFrame(f));
            }
        }
    }
    void schemeIsDistinctAndHarmless()
    {
        const auto s = defaultScheme();
        QCOMPARE(s.size(), std::size_t(24));
        QSet<int> seen;
        for (const auto &sc : s) {
            QVERIFY(sc.chord.usage >= 0x69 && sc.chord.usage <= 0x6e);  // F14..F19 only
            QVERIFY(sc.chord.mods == 0 || sc.chord.mods == mod::LShift || sc.chord.mods == mod::LCtrl || sc.chord.mods == mod::LAlt);
            const int key = sc.chord.mods << 8 | sc.chord.usage;
            QVERIFY(!seen.contains(key));
            seen.insert(key);
        }
        QCOMPARE(s[0].name, std::string("F14"));
        QCOMPARE(s[6].name, std::string("shift+F14"));
        QCOMPARE(s[23].name, std::string("alt+F19"));
    }
    // blob03: padclaude's measured frames, byte for byte.
    void blobFrames()
    {
        const Frame open = blob::openFrame();
        QCOMPARE(QString::fromStdString(hex(open, 4)), QStringLiteral("03 a1 01 00"));
        QCOMPARE(QString::fromStdString(hex(blob::closeFrame(), 4)), QStringLiteral("03 aa aa 00"));
        const Frame f14 = blob::record(1, Chord{0, 0x69});
        QCOMPARE(QString::fromStdString(hex(f14, 10)), QStringLiteral("03 01 00 00 69 00 00 00 00 00"));
        QCOMPARE(QString::fromStdString(hex(blob::record(24, Chord{mod::LAlt, 0x6e}), 6)), QStringLiteral("03 18 04 00 6e 00"));
        for (const Frame &f : {open, blob::closeFrame(), f14, blob::record(3, Chord{})}) {
            QVERIFY(blob::isAllowedFrame(f));
        }
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument, blob::record(0, Chord{0, 4}));
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument, blob::record(25, Chord{0, 4}));
        QVERIFY_THROWS_EXCEPTION(std::invalid_argument, blob::record(1, Chord{0, 0xe0}));
        auto with = [](std::initializer_list<std::pair<int, int>> bytes) {
            Frame f{};
            for (auto [i, v] : bytes) {
                f[std::size_t(i)] = std::uint8_t(v);
            }
            return f;
        };
        // Bootloader/variant opcodes, a missing marker, layer/LED commands and stray bytes are refused.
        for (const Frame &bad : {with({{0, 3}, {1, 0xfe}, {2, 1}}), with({{0, 3}, {1, 0xef}}), with({{0, 3}, {1, 0x5a}}), with({{0, 3}, {1, 0xfc}}),
                                 with({{0, 3}, {1, 0xb0}, {2, 0x18}}), with({{0, 3}, {1, 0xaa}, {2, 0xa1}}), with({{0, 3}, {1, 0xa1}, {2, 2}}),
                                 with({{0, 1}, {1, 1}, {4, 0x69}}), with({{0, 3}, {1, 1}, {3, 1}, {4, 0x69}}), with({{0, 3}, {1, 1}, {4, 0x69}, {9, 1}}),
                                 with({{0, 3}, {1, 0}, {4, 0x69}}), with({{0, 3}, {1, 0xaa}, {2, 0xaa}, {5, 1}})}) {
            QVERIFY2(!blob::isAllowedFrame(bad), hex(bad, 10).c_str());
        }
    }

    void slotRanges()
    {
        QCOMPARE(parseSlotRange("1-24"), (std::optional<std::pair<int, int>>{{1, 24}}));
        QCOMPARE(parseSlotRange("1-1"), (std::optional<std::pair<int, int>>{{1, 1}}));
        QCOMPARE(parseSlotRange("7"), (std::optional<std::pair<int, int>>{{7, 7}}));
        for (const char *bad : {"", "0", "25", "3-2", "1-25", "-3", "1-", "a", "1--2", "001", " 1"}) {
            QVERIFY2(!parseSlotRange(bad), bad);
        }
    }

    void numberingsCoverAllControls()
    {
        for (auto n : {Numbering::KeysThenKnobs, Numbering::VendorTwelve}) {
            QSet<QString> targets;
            for (std::uint8_t slot = 1; slot <= 24; ++slot) {
                const auto t = slotTarget(n, slot);
                targets.insert(QString::fromStdString(t.control + "." + t.role));
            }
            QCOMPARE(targets.size(), 24);
            QVERIFY(targets.contains(QStringLiteral("key15.key")));
            QVERIFY(targets.contains(QStringLiteral("knob3.cw")));
        }
        QCOMPARE(slotTarget(Numbering::KeysThenKnobs, 16).control, std::string("knob1"));
        QCOMPARE(slotTarget(Numbering::KeysThenKnobs, 16).role, std::string("ccw"));
        QCOMPARE(slotTarget(Numbering::KeysThenKnobs, 17).role, std::string("press"));
        QCOMPARE(slotTarget(Numbering::VendorTwelve, 13).control, std::string("knob1"));
        QCOMPARE(slotTarget(Numbering::VendorTwelve, 22).control, std::string("key13"));
    }
    void descriptorsOfThisPad()
    {
        // Captured from /sys/class/hidraw/hidraw{4,5}/device/report_descriptor of key153.
        const auto if1 = bytes({0x05, 0x01, 0x09, 0x00, 0xa1, 0x01, 0x15, 0x00, 0x25, 0xff, 0x19, 0x01, 0x29, 0x08, 0x95, 0x08, 0x75, 0x08,
                                0x81, 0x02, 0x09, 0x02, 0x15, 0x00, 0x25, 0xff, 0x75, 0x08, 0x95, 0x40, 0x91, 0x06, 0xc0});
        const auto info = parseDescriptor(if1);
        QVERIFY(info.ok);
        QCOMPARE(info.outputBits, 512);
        QCOMPARE(info.inputBits, 64);
        QVERIFY(!info.hasReportIds);
        QCOMPARE(generationFor(info), std::optional<Generation>(Generation::Rid0));

        const auto if0 = bytes({0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01,
                                0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x03, 0x75, 0x01, 0x05, 0x08, 0x19, 0x01,
                                0x29, 0x03, 0x91, 0x02, 0x95, 0x05, 0x75, 0x01, 0x91, 0x01, 0x95, 0x06, 0x75, 0x08, 0x26, 0xff, 0x00, 0x05,
                                0x07, 0x19, 0x00, 0x29, 0x91, 0x81, 0x00, 0xc0});
        QVERIFY(!generationFor(parseDescriptor(if0)));  // boot keyboard: never a config target

        // Report-id-3 generation as documented by other projects.
        const auto rid3 = bytes({0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x03, 0x09, 0x02, 0x15, 0x00, 0x26, 0x00, 0xff, 0x75, 0x08,
                                 0x95, 0x40, 0x81, 0x06, 0x09, 0x02, 0x75, 0x08, 0x95, 0x40, 0x91, 0x06, 0xc0});
        QCOMPARE(generationFor(parseDescriptor(rid3)), std::optional<Generation>(Generation::Rid3));
        QVERIFY(!parseDescriptor(bytes({0x75})).ok);  // truncated item
    }
};

QTEST_GUILESS_MAIN(TestProto)
#include "tst_proto.moc"
