#include "circuit.h"
#include "solver.h"
#include "waves.h"
#include <QCoreApplication>
#include <QFileInfo>
#include <QTextStream>

int main(int argc, char** argv) {
    QCoreApplication app(argc, argv);
    const auto args = app.arguments();
    try {
        if (args.size()==4 && args[1]=="import-csv") {
            importCsv(args[2],args[3]);
            return 0;
        }
        if (args.size() == 4 && args[1] == "run") {
            if (QFileInfo(args[2]) == QFileInfo(args[3]))
                throw std::runtime_error("result path must differ from input circuit");
            simulateCsv(Circuit::load(args[2]), args[3]);
            return 0;
        }
        if (args.size() == 4 && args[1] == "copy") {
            Circuit::load(args[2]).save(args[3]);
            return 0;
        }
        if (args.size() == 3 && args[1] == "inspect") {
            const auto circuit = Circuit::load(args[2]);
            QTextStream(stdout) << "unconnected pins: " << circuit.missingPins().join(", ") << Qt::endl;
            return 0;
        }
        QTextStream(stderr) << "usage: litzsim-cli inspect FILE | copy INPUT OUTPUT | run INPUT CSV\n";
        return 2;
    } catch (const std::exception& e) {
        QTextStream(stderr) << e.what() << Qt::endl;
        return 1;
    }
}
