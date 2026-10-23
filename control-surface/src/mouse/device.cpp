// SPDX-License-Identifier: GPL-2.0-or-later
#include "device.h"
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QSet>
#include <array>
#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <linux/uinput.h>
#include <sys/ioctl.h>
#include <unistd.h>

namespace cs::mouse {
namespace {
template <size_t N> using Bits = std::array<unsigned char, (N + 7) / 8>;
template <size_t N> bool bit(const std::array<unsigned char, N> &bits, int code) {
    return code >= 0 && size_t(code / 8) < N && (bits[size_t(code / 8)] & (1 << (code % 8)));
}
bool error(QString *out, const QString &operation) {
    *out = operation + QStringLiteral(": ") + QString::fromLocal8Bit(strerror(errno));
    return false;
}
QString text(const QString &path) {
    QFile file(path);
    return file.open(QIODevice::ReadOnly) ? QString::fromUtf8(file.readAll()).trimmed() : QString();
}
QStringList candidates() {
    QStringList paths;
    for (const auto &entry : QDir(QStringLiteral("/sys/class/input")).entryList({QStringLiteral("event*")}, QDir::Dirs)) {
        const QString root = QStringLiteral("/sys/class/input/") + entry + QStringLiteral("/device");
        if (text(root + "/id/vendor") == "1532" && text(root + "/id/product") == "00b4")
            paths << QStringLiteral("/dev/input/") + entry;
    }
    return paths;
}
bool held(int fd, QString *out) {
    Bits<KEY_CNT> keys{};
    if (ioctl(fd, EVIOCGKEY(keys.size()), keys.data()) < 0) {
        error(out, QStringLiteral("Read held buttons"));
        return true;
    }
    for (auto byte : keys)
        if (byte) {
            *out = QStringLiteral("Release all mouse buttons before remapping starts");
            return true;
        }
    return false;
}
} // namespace

VirtualDevice::~VirtualDevice() {
    if (m_fd >= 0) {
        ioctl(m_fd, UI_DEV_DESTROY);
        ::close(m_fd);
    }
}

bool VirtualDevice::create(const QString &name, QString *out) {
    uinput_setup setup{};
    setup.id = {BUS_USB, 0x1d6b, 0x0cf3, 1};
    const auto bytes = name.toUtf8();
    qstrncpy(setup.name, bytes.constData(), sizeof(setup.name));
    if (ioctl(m_fd, UI_DEV_SETUP, &setup) < 0 || ioctl(m_fd, UI_DEV_CREATE) < 0)
        return error(out, QStringLiteral("Create virtual mouse input"));
    return true;
}

bool VirtualDevice::clone(int source, const QString &name, QString *out) {
    m_fd = ::open("/dev/uinput", O_WRONLY | O_NONBLOCK | O_CLOEXEC);
    if (m_fd < 0)
        return error(out, QStringLiteral("Open /dev/uinput"));
    struct Family {
        int type, max, request;
    };
    for (const Family family : {Family{EV_KEY, KEY_MAX, UI_SET_KEYBIT},
                                {EV_REL, REL_MAX, UI_SET_RELBIT},
                                {EV_ABS, ABS_MAX, UI_SET_ABSBIT},
                                {EV_MSC, MSC_MAX, UI_SET_MSCBIT},
                                {EV_SW, SW_MAX, UI_SET_SWBIT}}) {
        Bits<KEY_CNT> codes{};
        if (ioctl(source, EVIOCGBIT(family.type, codes.size()), codes.data()) < 0)
            return error(out, QStringLiteral("Read input capabilities"));
        bool enabled = false;
        for (int code = 0; code <= family.max; ++code) {
            if (!bit(codes, code))
                continue;
            if ((!enabled && ioctl(m_fd, UI_SET_EVBIT, family.type) < 0) || ioctl(m_fd, family.request, code) < 0)
                return error(out, QStringLiteral("Copy input capabilities"));
            enabled = true;
            if (family.type == EV_ABS) {
                uinput_abs_setup abs{};
                abs.code = code;
                if (ioctl(source, EVIOCGABS(code), &abs.absinfo) < 0 || ioctl(m_fd, UI_ABS_SETUP, &abs) < 0)
                    return error(out, QStringLiteral("Copy absolute axis"));
            }
        }
    }
    Bits<INPUT_PROP_CNT> properties{};
    if (ioctl(source, EVIOCGPROP(properties.size()), properties.data()) < 0)
        return error(out, QStringLiteral("Read input properties"));
    for (int code = 0; code <= INPUT_PROP_MAX; ++code)
        if (bit(properties, code) && ioctl(m_fd, UI_SET_PROPBIT, code) < 0)
            return error(out, QStringLiteral("Copy input properties"));
    return create(name, out);
}

bool VirtualDevice::actions(bool pointer, QString *out) {
    m_fd = ::open("/dev/uinput", O_WRONLY | O_NONBLOCK | O_CLOEXEC);
    if (m_fd < 0)
        return error(out, QStringLiteral("Open action /dev/uinput"));
    if (ioctl(m_fd, UI_SET_EVBIT, EV_KEY) < 0)
        return error(out, QStringLiteral("Enable action keys"));
    if (pointer) {
        if (ioctl(m_fd, UI_SET_EVBIT, EV_REL) < 0)
            return error(out, QStringLiteral("Enable action pointer"));
        for (int code : {BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA})
            if (ioctl(m_fd, UI_SET_KEYBIT, code) < 0)
                return error(out, QStringLiteral("Enable action button"));
        for (int code : {REL_X, REL_Y, REL_WHEEL, REL_HWHEEL})
            if (ioctl(m_fd, UI_SET_RELBIT, code) < 0)
                return error(out, QStringLiteral("Enable action axis"));
    } else {
        for (int code = 1; code < BTN_MISC; ++code)
            if (ioctl(m_fd, UI_SET_KEYBIT, code) < 0)
                return error(out, QStringLiteral("Enable keyboard action"));
        for (const auto &name : keyNames()) {
            const int code = keyCodeFromName(name);
            if (code > 0 && ioctl(m_fd, UI_SET_KEYBIT, code) < 0)
                return error(out, QStringLiteral("Enable media action"));
        }
    }
    return create(pointer ? QStringLiteral("smpl mouse remapper actions pointer") : QStringLiteral("smpl mouse remapper actions keyboard"),
                  out);
}

bool VirtualDevice::write(const InputTriple &triple, QString *out) {
    input_event event{};
    event.type = triple.type;
    event.code = triple.code;
    event.value = triple.value;
    ssize_t size;
    do {
        size = ::write(m_fd, &event, sizeof(event));
    } while (size < 0 && errno == EINTR);
    return size == sizeof(event) || error(out, QStringLiteral("Write virtual input"));
}

Devices::Node::~Node() {
    notifier.reset();
    if (fd >= 0) {
        ioctl(fd, EVIOCGRAB, 0);
        ::close(fd);
    }
}

Devices::Devices(Router &router) : m_router(router) {
    router.output = [this](bool pointer, const InputTriple &event) {
        auto *device = pointer ? m_pointer.get() : m_keyboard.get();
        QString reason;
        if (!device || !device->write(event, &reason))
            fail(reason.isEmpty() ? QStringLiteral("Action device unavailable") : reason);
    };
}

Devices::~Devices() { close(); }
void Devices::close() {
    m_router.disconnected();
    m_nodes.clear();
    m_pointer.reset();
    m_keyboard.reset();
}

bool Devices::present() const { return !candidates().isEmpty(); }

bool Devices::attach(QString *out) {
    close();
    auto paths = candidates();
    if (paths.isEmpty()) {
        *out = QStringLiteral("Razer Naga V2 HyperSpeed (1532:00b4) not connected");
        return false;
    }
    std::vector<std::unique_ptr<Node>> nodes;
    QSet<QString> physical;
    for (const auto &path : paths) {
        const QString root = QStringLiteral("/sys/class/input/") + QFileInfo(path).fileName() + QStringLiteral("/device");
        bool ok = false;
        const quint64 types = text(root + "/capabilities/ev").toULongLong(&ok, 16);
        if (!ok || !(types & (1 << EV_KEY)))
            continue; // The absolute-only auxiliary interface is not a button input.
        auto node = std::make_unique<Node>();
        node->path = path;
        node->fd = ::open(path.toLocal8Bit().constData(), O_RDONLY | O_NONBLOCK | O_CLOEXEC);
        if (node->fd < 0)
            return error(out, QStringLiteral("Open ") + path + QStringLiteral(" (mouse input access required)"));
        input_id id{};
        char phys[256]{};
        Bits<KEY_CNT> keys{};
        if (ioctl(node->fd, EVIOCGID, &id) < 0 || ioctl(node->fd, EVIOCGPHYS(sizeof(phys)), phys) < 0 ||
            ioctl(node->fd, EVIOCGBIT(EV_KEY, keys.size()), keys.data()) < 0)
            return error(out, QStringLiteral("Verify ") + path);
        if (id.vendor != Vendor || id.product != Product) {
            *out = QStringLiteral("Device identity changed during discovery");
            return false;
        }
        const QString location = QString::fromLocal8Bit(phys);
        const int slash = location.lastIndexOf('/');
        if (slash < 0 || !location.mid(slash + 1).startsWith("input")) {
            *out = QStringLiteral("Mouse has no stable physical interface identity");
            return false;
        }
        physical.insert(location.left(slash));
        node->pointer = bit(keys, BTN_LEFT);
        node->source = location.mid(slash + 1) + (node->pointer ? "/pointer" : "/keyboard");
        for (const auto &previous : nodes)
            if (previous->source == node->source) {
                *out = QStringLiteral("Ambiguous mouse input interfaces");
                return false;
            }
        if (held(node->fd, out) || !node->mirror.clone(node->fd, QStringLiteral("smpl Razer passthrough ") + node->source, out))
            return false;
        nodes.push_back(std::move(node));
    }
    if (physical.size() != 1 || nodes.empty()) {
        *out = QStringLiteral("Expected exactly one Razer Naga V2 HyperSpeed; "
                              "multiple devices are not supported");
        return false;
    }
    m_keyboard = std::make_unique<VirtualDevice>();
    m_pointer = std::make_unique<VirtualDevice>();
    if (!m_keyboard->actions(false, out) || !m_pointer->actions(true, out)) {
        close();
        return false;
    }
    for (auto &node : nodes) {
        input_event events[64];
        ssize_t size;
        do {
            size = ::read(node->fd, events, sizeof(events));
        } while (size > 0 || (size < 0 && errno == EINTR));
        if (size < 0 && errno != EAGAIN) {
            error(out, QStringLiteral("Drain ") + node->path);
            close();
            return false;
        }
        if (held(node->fd, out)) {
            close();
            return false;
        }
        if (ioctl(node->fd, EVIOCGRAB, 1) < 0) {
            error(out, QStringLiteral("Grab ") + node->path);
            close();
            return false;
        }
        if (held(node->fd, out)) {
            close();
            return false;
        }
    }
    m_nodes = std::move(nodes);
    for (auto &node : m_nodes) {
        node->notifier = std::make_unique<QSocketNotifier>(node->fd, QSocketNotifier::Read, this);
        connect(node->notifier.get(), &QSocketNotifier::activated, this, [this, ptr = node.get()] { read(*ptr); });
    }
    return true;
}

void Devices::fail(const QString &reason) {
    // A queued teardown avoids destroying a notifier/router during its callback.
    for (auto &node : m_nodes)
        node->notifier->setEnabled(false);
    if (failed)
        failed(reason);
}

void Devices::read(Node &node) {
    input_event events[64];
    for (;;) {
        const ssize_t size = ::read(node.fd, events, sizeof(events));
        if (size < 0 && errno == EINTR)
            continue;
        if (size < 0 && errno == EAGAIN)
            return;
        if (size <= 0 || size % sizeof(input_event)) {
            fail(QStringLiteral("Mouse disconnected or input read failed"));
            return;
        }
        for (size_t i = 0; i < size_t(size) / sizeof(input_event); ++i) {
            const auto &event = events[i];
            if (event.type == EV_SYN && event.code == SYN_DROPPED) {
                fail(QStringLiteral("Mouse input overflow; releasing held actions and reconnecting"));
                return;
            }
            if (m_router.consume(node.source, node.pointer, {event.type, event.code, event.value}))
                continue;
            if (event.type == EV_LED || event.type == EV_REP)
                continue;
            QString reason;
            if (!node.mirror.write({event.type, event.code, event.value}, &reason)) {
                fail(reason);
                return;
            }
        }
    }
}

} // namespace cs::mouse
