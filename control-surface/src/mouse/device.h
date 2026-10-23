// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "mapping.h"
#include <QSocketNotifier>
#include <memory>
#include <vector>

namespace cs::mouse {

class VirtualDevice {
public:
    ~VirtualDevice();
    bool clone(int source, const QString &name, QString *error);
    bool actions(bool pointer, QString *error);
    bool write(const InputTriple &event, QString *error);

private:
    int m_fd = -1;
    bool create(const QString &name, QString *error);
};

class Devices : public QObject {
public:
    explicit Devices(Router &router);
    ~Devices() override;
    bool attach(QString *error);
    void close();
    bool connected() const { return !m_nodes.empty(); }
    bool present() const;
    std::function<void(const QString &)> failed;

private:
    struct Node {
        ~Node();
        int fd = -1;
        QString path, source;
        bool pointer = false;
        VirtualDevice mirror;
        std::unique_ptr<QSocketNotifier> notifier;
    };
    Router &m_router;
    std::vector<std::unique_ptr<Node>> m_nodes;
    std::unique_ptr<VirtualDevice> m_keyboard, m_pointer;
    void read(Node &node);
    void fail(const QString &error);
};

} // namespace cs::mouse
