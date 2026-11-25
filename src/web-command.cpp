#include "web-command.h"
#include "circuit.h"
#include "studio.h"
#include "waves.h"
#include <QApplication>
#include <QCryptographicHash>
#include <QElapsedTimer>
#include <QFile>
#include <QJsonDocument>
#include <cstdio>
#include <stdexcept>

namespace {
void require(bool valid,const char* message) {
    if (!valid) throw std::runtime_error(message);
}

QString hash(const QByteArray& bytes) {
    return QString::fromLatin1(QCryptographicHash::hash(bytes,QCryptographicHash::Sha256).toHex());
}

QString revision(const QJsonObject& document) {
    return hash(QJsonDocument(document).toJson(QJsonDocument::Compact));
}

QByteArray readCircuit(const QString& path) {
    QFile file(path);
    require(file.open(QIODevice::ReadOnly),"Cannot read circuit file.");
    return file.readAll();
}

void validatePin(const Circuit& circuit,const QString& pin) {
    const auto dot=pin.indexOf('.');
    const auto type=circuit.document["parts"].toObject()[pin.left(dot)].toObject()["type"].toString();
    require(dot>0 && library().value(type).pins.contains(pin.mid(dot+1)),"Unknown terminal.");
}

int gridCoordinate(const QJsonObject& operation,const char* key) {
    const auto coordinate=operation[key];
    require(coordinate.isDouble() && coordinate.toDouble()==coordinate.toInt() &&
            qAbs(coordinate.toDouble())<=200,"Grid coordinate outside the spike drawing area.");
    return coordinate.toInt();
}

void apply(Circuit& circuit,const QJsonObject& operation,QJsonArray& created) {
    const auto action=operation["action"].toString(),id=operation["id"].toString();
    if (action=="add")
        created.append(circuit.insert(operation["type"].toString(),gridCoordinate(operation,"x"),gridCoordinate(operation,"y")));
    else if (action=="move")
        circuit.move(id,gridCoordinate(operation,"x"),gridCoordinate(operation,"y"));
    else if (action=="rotate") circuit.rotate(id);
    else if (action=="setParameter") {
        require(operation["value"].isDouble(),"Expected a numeric parameter value.");
        circuit.setParameter(id,operation["key"].toString(),operation["value"].toDouble());
    } else if (action=="connect") {
        const auto a=operation["a"].toString(),b=operation["b"].toString();
        validatePin(circuit,a);validatePin(circuit,b);circuit.connectPins(a,b);
    } else if (action=="disconnect") {
        validatePin(circuit,id);circuit.disconnectPin(id);
    } else if (action=="delete") {
        require(circuit.document["parts"].toObject().contains(id),"Unknown part.");
        circuit.erase(id);
    } else throw std::runtime_error("Unknown circuit operation.");
}

QJsonObject request(const QJsonObject& input) {
    QElapsedTimer timer;
    timer.start();
    const auto action=input["action"].toString();
    const auto path=input["path"].toString();
    Circuit circuit{input["document"].toObject()};
    QJsonObject result;
    if (action=="open") {
        const auto bytes=readCircuit(path);
        circuit.document=QJsonDocument::fromJson(bytes).object();
        result["diskRevision"]=hash(bytes);
    }
    circuit.validate();
    if (action=="save") {
        require(hash(readCircuit(path))==input["diskRevision"].toString(),"Circuit changed on disk. Reopen it before saving.");
        circuit.save(path);
        return {{"diskRevision",hash(QJsonDocument(circuit.document).toJson())}};
    }
    if (action=="snapshot") {
        if (input.contains("stopTime_s")) {
            auto simulation=circuit.document["simulation"].toObject();
            simulation["stopTime_s"]=input["stopTime_s"];
            circuit.document["simulation"]=simulation;
        }
        circuit.save(path);
        return {};
    }

    const auto before=revision(circuit.document);
    QJsonArray types,created;
    for (const auto& type:library().keys()) types.append(type);
    if (action=="apply") {
        require(input["expectedRevision"]==before,"Circuit revision changed.");
        const auto operations=input["operations"];
        require(operations.isArray(),"Expected an array of edit operations.");
        int index=0;
        for (const auto& operation:operations.toArray()) {
            try { apply(circuit,operation.toObject(),created); }
            catch (const std::exception& e) {
                throw std::runtime_error("Operation "+std::to_string(index)+": "+e.what());
            }
            ++index;
        }
        circuit.validate();
    } else require(action=="open" || action=="layout","Unknown circuit request.");
    const auto after=revision(circuit.document);
    const double applyMs=timer.nsecsElapsed()/1e6;
    timer.restart();
    result["layout"]=Sheet::layout(circuit);
    result["timing"]=QJsonObject{{"applyMs",applyMs},{"layoutMs",timer.nsecsElapsed()/1e6}};
    result["document"]=circuit.document;
    result["revision"]=after;
    result["receipt"]=QJsonObject{{"before",before},{"after",after},{"created",created}};
    result["types"]=types;
    return result;
}

void reply(const QJsonObject& response) {
    auto bytes=QJsonDocument(response).toJson(QJsonDocument::Compact);
    std::fwrite(bytes.constData(),1,bytes.size(),stdout);
    std::fputc('\n',stdout);
    std::fflush(stdout);
}
}

int webCircuitCommand(int argc,char** argv) {
    qputenv("QT_QPA_PLATFORM","offscreen");
    QApplication app(argc,argv);
    QApplication::setFont(studioFont());
    QFile input;
    input.open(stdin,QIODevice::ReadOnly);
    while (!input.atEnd()) {
        try {
            const auto parsed=QJsonDocument::fromJson(input.readLine());
            require(parsed.isObject(),"Expected a JSON request.");
            reply({{"value",request(parsed.object())}});
        } catch (const std::exception& e) {
            reply({{"error",QString::fromUtf8(e.what())}});
        }
        if (argc==2) break;
    }
    return 0;
}

int webWaveCommand(const QString& file,int channel) {
    Waves waves(file);
    require(channel>0 && channel<waves.channels.size(),"Invalid waveform channel.");
    const auto from=waves.at(0,0),to=waves.at(waves.rows-1,0);
    QJsonArray points,channels;
    for (auto p:waves.envelope(from,to,channel,1200)) points.append(QJsonArray{p.x(),p.y()});
    for (auto name:waves.channels) channels.append(name);
    reply({{"channels",channels},{"points",points},{"from",from},{"to",to}});
    return 0;
}
