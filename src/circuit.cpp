#include "circuit.h"
#include <QFile>
#include <QSaveFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSet>
#include <QRegularExpression>
#include <cmath>
#include <stdexcept>

static void require(bool ok, const QString& message) {
    if (!ok) throw std::runtime_error(message.toStdString());
}
const QMap<QString, Part>& library() {
    static const QMap<QString, Part> parts = {
        {"dc", {{"p","n"}, {{"voltage_V",5}}}},
        {"sine", {{"p","n"}, {{"amplitude_V",1},{"frequency_Hz",50}}}},
        {"resistor", {{"p","n"}, {{"resistance_ohm",1000}}}},
        {"inductor", {{"p","n"}, {{"inductance_H",1e-3},{"initialCurrent_A",0}}}},
        {"capacitor", {{"p","n"}, {{"capacitance_F",1e-6},{"initialVoltage_V",0}}}},
        {"ground", {{"ref"}, {}}},
        {"diode", {{"a","k"}, {}}},
        {"nmos", {{"d","g","s"}, {}}},
        {"pwm", {{"out","ref"}, {{"frequency_Hz",10000},{"duty",0.5},{"high_V",10}}}},
        {"voltmeter", {{"p","n"}, {}}},
        {"ammeter", {{"p","n"}, {}}}
    };
    return parts;
}
Circuit Circuit::load(const QString& path) {
    QFile file(path);
    require(file.open(QIODevice::ReadOnly), file.errorString());
    QJsonParseError error;
    auto parsed = QJsonDocument::fromJson(file.readAll(), &error);
    require(parsed.isObject(), path + ": " + error.errorString());
    Circuit circuit{parsed.object()};
    circuit.validate();
    return circuit;
}
void Circuit::save(const QString& path) const {
    validate();
    QSaveFile file(path);
    require(file.open(QIODevice::WriteOnly), file.errorString());
    auto bytes = QJsonDocument(document).toJson();
    require(file.write(bytes) == bytes.size(), file.errorString());
    require(file.commit(), file.errorString());
}
static bool identifier(const QString& s) {
    static const QRegularExpression pattern("^[A-Za-z_][A-Za-z_0-9-]*$");
    return pattern.match(s).hasMatch();
}
void Circuit::validate() const {
    require(document["version"].toDouble() == 1, "unsupported circuit version");
    for (const auto key : {"parts","nets","layout","simulation"})
        require(document[key].isObject(), QString("expected object: ") + key);
    const auto parts = document["parts"].toObject();
    QSet<QString> terminals;
    for (auto it = parts.begin(); it != parts.end(); ++it) {
        const auto part = it.value().toObject();
        const QString type = part["type"].toString();
        require(identifier(it.key()) && library().contains(type), "invalid part: " + it.key());
        const auto def = library().value(type);
        require(part["parameters"].isObject(), "missing parameters: " + it.key());
        const auto parameters = part["parameters"].toObject();
        require(parameters.keys() == def.defaults.keys(), "invalid parameters: " + it.key());
        for (const auto& value : parameters)
            require(value.isDouble() && std::isfinite(value.toDouble()), "invalid value: " + it.key());
        for (const auto& pin : def.pins) terminals.insert(it.key() + "." + pin);
    }
    const auto nets = document["nets"].toObject();
    for (auto it = nets.begin(); it != nets.end(); ++it) {
        require(identifier(it.key()) && it.value().isArray(), "invalid net: " + it.key());
        for (const auto& pin : it.value().toArray())
            require(terminals.remove(pin.toString()), "unknown or multiply connected pin: " + pin.toString());
    }
    const auto layout = document["layout"].toObject();
    for (auto it = layout.begin(); it != layout.end(); ++it) {
        require(parts.contains(it.key()) && it.value().isObject(), "invalid layout: " + it.key());
        for (const auto key : {"x","y"}) {
            auto coord = it.value().toObject()[key];
            require(coord.isDouble() && coord.toDouble() == coord.toInt(), "invalid grid coordinate: " + it.key());
        }
    }
    const auto sim = document["simulation"].toObject();
    for (const auto key : {"timeStep_s","stopTime_s"})
        require(sim[key].isDouble() && sim[key].toDouble() > 0, QString("invalid ") + key);
}
QStringList Circuit::missingPins() const {
    QSet<QString> connected;
    for (const auto& net : document["nets"].toObject())
        for (const auto& pin : net.toArray()) connected.insert(pin.toString());
    QStringList missing;
    const auto parts = document["parts"].toObject();
    for (auto it = parts.begin(); it != parts.end(); ++it)
        for (const auto& pin : library().value(it.value().toObject()["type"].toString()).pins) {
            QString name = it.key() + "." + pin;
            if (!connected.contains(name)) missing << name;
        }
    return missing;
}
void Circuit::move(const QString& id, int x, int y) {
    require(document["parts"].toObject().contains(id), "unknown part: " + id);
    auto layout = document["layout"].toObject();
    auto position = layout[id].toObject();
    position["x"] = x;
    position["y"] = y;
    layout[id] = position;
    document["layout"] = layout;
}
