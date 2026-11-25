#include "circuit.h"
#include <QJsonArray>
#include <QTemporaryDir>
#include <QDebug>
#include <stdexcept>
static void check(bool ok) { if (!ok) throw std::runtime_error("circuit contract failed"); }
int main(int argc, char** argv) {
    if (argc != 2) return 2;
    QTemporaryDir dir;
    for (const auto name : {"rc","buck"}) {
        auto circuit = Circuit::load(QString::fromLocal8Bit(argv[1]) + "/" + name + ".litzsim");
        check(circuit.missingPins().isEmpty());
        auto nets = circuit.document["nets"];
        circuit.move("V1", 14, -2);
        auto path = dir.filePath("copy.litzsim");
        circuit.save(path);
        check(Circuit::load(path).document == circuit.document && circuit.document["nets"] == nets);
        auto broken = nets.toObject();
        broken["duplicate"] = QJsonArray{"V1.p"};
        circuit.document["nets"] = broken;
        bool rejected = false;
        try { circuit.validate(); } catch (const std::exception&) { rejected = true; }
        check(rejected);
        circuit.document["nets"] = QJsonObject{};
        circuit.save(path);
        check(Circuit::load(path).missingPins().contains("V1.p"));
    }
}
