#include "circuit.h"
#include "solver.h"
#include "studio.h"
#include "waves.h"
#include "wave-spike.h"
#include <QApplication>
#include <QFileInfo>
#include <QTextStream>
#include <cstring>

namespace {
int exportSchematic(int argc,char** argv) {
#ifndef Q_OS_WIN
    if(qEnvironmentVariableIsEmpty("QT_QPA_PLATFORM")) qputenv("QT_QPA_PLATFORM","offscreen");
#endif
    QApplication app(argc,argv);
    const auto args=app.arguments();
    bool ok=false;
    const double width=args.value(5).toDouble(&ok);
    const bool annotations=args.size()==7 && args[6]=="--annotations";
    if((args.size()!=6 && !annotations) || args[4]!="--width-mm" || !ok) {
        QTextStream(stderr) << "usage: litzsim-cli export INPUT OUTPUT.svg|.pdf --width-mm WIDTH [--annotations]\n";
        return 2;
    }
    QApplication::setFont(studioFont());
    Sheet sheet(args[2],Circuit::load(args[2]),nullptr);
    sheet.exportSchematic(args[3],width,annotations);
    return 0;
}
}

int main(int argc, char** argv) {
    try {
        if(argc>1 && std::strcmp(argv[1],"export")==0) return exportSchematic(argc,argv);
        QCoreApplication app(argc, argv);
        const auto args = app.arguments();
        if (args.size()==3 && args[1]=="wave-serve") return serveWave(args[2]);
        if (args.size()==4 && args[1]=="import-csv") {
            importCsv(args[2],args[3]);
            return 0;
        }
        if (args.size() == 4 && args[1] == "run") {
            if (QFileInfo(args[2]) == QFileInfo(args[3]))
                throw std::runtime_error("result path must differ from input circuit");
            auto circuit=Circuit::load(args[2]);
            if (args[3].endsWith(".wave",Qt::CaseInsensitive)) simulateWave(circuit,args[3]);
            else simulateCsv(circuit,args[3]);
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
        QTextStream(stderr) << "usage: litzsim-cli inspect FILE | copy INPUT OUTPUT | run INPUT CSV\n"
                               "       litzsim-cli wave-serve FILE.wave\n"
                               "       litzsim-cli export INPUT OUTPUT.svg|.pdf --width-mm WIDTH [--annotations]\n";
        return 2;
    } catch (const std::exception& e) {
        QTextStream(stderr) << e.what() << Qt::endl;
        return 1;
    }
}
