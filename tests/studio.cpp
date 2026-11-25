#include "studio.h"
#include "waves.h"
#include "solver.h"
#include <QEventLoop>
#include <QTimer>
#include <QJsonDocument>
#include <QApplication>
#include <QTemporaryDir>
#include <QGraphicsItem>
#include <algorithm>

namespace {
QPointF terminal(const Sheet& sheet,const QString& name) {
    for(auto* item:sheet.scene()->items())
        if(item->data(1).toString()==name) return item->scenePos();
    throw std::runtime_error(("missing scene terminal: "+name).toStdString());
}

void verifyMark(const Sheet& sheet,const QString& id,const QString& text,const QString& pin) {
    int matches=0;
    const auto target=terminal(sheet,id+"."+pin);
    for(auto* item:sheet.scene()->items()) {
        const auto* label=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
        if(!label || label->data(0).toString()!=id || label->text()!=text) continue;
        const auto area=label->sceneBoundingRect();
        const auto nearest=QPointF(std::clamp(target.x(),area.left(),area.right()),
                                   std::clamp(target.y(),area.top(),area.bottom()));
        if(QLineF(nearest,target).length()<18 && label->brush().color()==Qt::black) ++matches;
    }
    if(matches!=1) throw std::runtime_error(("missing or misplaced mark: "+id+" "+text).toStdString());
}

void verifySymbol(const Sheet& sheet,const QString& id,const QStringList& pins,bool current=false) {
    for(auto* item:sheet.scene()->items()) {
        const auto* symbol=qgraphicsitem_cast<QGraphicsPathItem*>(item);
        if(!symbol || symbol->data(0).toString()!=id) continue;
        const auto path=symbol->path();
        QList<QPointF> ends;
        QList<QLineF> strokes;
        QLineF shaft;
        for(int i=0;i<path.elementCount();++i) {
            const auto point=path.elementAt(i);
            ends.append(symbol->mapToScene(point.x,point.y));
            if(!current || !point.isLineTo() || i==0) continue;
            const auto start=path.elementAt(i-1);
            const QLineF stroke(ends[i-1],ends.last());
            strokes.append(stroke);
            if(start.x*point.x<0 && start.y==point.y && stroke.length()>15) shaft=stroke;
        }
        bool connected=true;
        for(const auto& pin:pins)
            if(!ends.contains(terminal(sheet,id+"."+pin))) connected=false;
        if(!connected || symbol->pen().color()!=Qt::black) continue;
        if(!current) return;
        const auto direction=terminal(sheet,id+".n")-terminal(sheet,id+".p");
        if(QPointF::dotProduct(direction,shaft.p2()-shaft.p1())<0)
            shaft=QLineF(shaft.p2(),shaft.p1());
        // Two angled strokes must meet at the tip on the n side of the shaft.
        const auto wings=std::count_if(strokes.begin(),strokes.end(),[&](QLineF stroke){
            if(stroke.p2()==shaft.p2()) stroke=QLineF(stroke.p2(),stroke.p1());
            const auto wing=stroke.p2()-stroke.p1();
            return stroke.p1()==shaft.p2() && QPointF::dotProduct(direction,wing)<0 &&
                   (direction.x()*wing.y()-direction.y()*wing.x())!=0;
        });
        if(wings==2) return;
    }
    throw std::runtime_error(("symbol disagrees with its terminals: "+id).toStdString());
}

void verifySchematic(const QString& examples) {
    for(const QString name:{"rc","buck","pfc"}) {
        const auto path=examples+"/"+name+".litzsim";
        Sheet sheet(path,Circuit::load(path),nullptr);
        const QString probe=name=="pfc" ? "Vdc" : "VC";
        verifyMark(sheet,probe,"+","p");
        for(auto* item:sheet.scene()->items()) {
            const auto* wire=qgraphicsitem_cast<QGraphicsLineItem*>(item);
            const auto* junction=qgraphicsitem_cast<QGraphicsEllipseItem*>(item);
            const auto* text=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
            const auto* symbol=qgraphicsitem_cast<QGraphicsPathItem*>(item);
            if((wire && wire->pen().color()!=Qt::black) ||
               (junction && junction->brush().color()!=Qt::black) ||
               (symbol && symbol->pen().color()!=Qt::black) ||
               (text && text->brush().color()!=Qt::black))
                throw std::runtime_error(("unexpected diagram color or routing warning: "+name).toStdString());
        }
        const auto gates=sheet.circuit.document["control"].toObject()["gateSignals"].toObject();
        QStringList transistors;
        for(auto it=gates.begin();it!=gates.end();++it) {
            const auto pin=it.value().toString();
            transistors.append(pin.section('.',0,0));
            const auto end=terminal(sheet,pin);
            for(auto* item:sheet.scene()->items()) {
                const auto* wire=qgraphicsitem_cast<QGraphicsLineItem*>(item);
                if(wire && (wire->line().p1()==end || wire->line().p2()==end))
                    throw std::runtime_error("controller mapping drawn as a physical wire");
            }
        }
        if(name=="buck") transistors.append("Q1");
        const QStringList sensors=name=="pfc" ? QStringList{"IL1","IL2"} : QStringList{};
        for(int turn=0;turn<4;++turn) {
            verifySymbol(sheet,probe,{"p","n"});
            verifyMark(sheet,probe,"+","p");
            verifyMark(sheet,probe,"−","n");
            for(const auto& id:sensors) verifySymbol(sheet,id,{"p","n"},true);
            for(const auto& id:transistors) {
                verifySymbol(sheet,id,{"d","g","s"});
                verifyMark(sheet,id,"D","d");
                verifyMark(sheet,id,"G","g");
                verifyMark(sheet,id,"S","s");
            }
            for(auto it=gates.begin();it!=gates.end();++it)
                verifyMark(sheet,it.value().toString().section('.',0,0),"제어 "+it.key(),"g");
            sheet.circuit.rotate(probe);
            for(const auto& id:sensors+transistors) sheet.circuit.rotate(id);
            sheet.redraw();
        }
    }
}
}

int main(int argc,char** argv) {
    QApplication app(argc,argv);
    if (argc!=2) return 2;
    QTemporaryDir dir;
    Studio studio(dir.path());
    QString examples=QString::fromLocal8Bit(argv[1]);
    try {verifySchematic(examples);}
    catch(const std::exception& error) {qWarning()<<error.what();return 8;}
    QString a=dir.filePath("a.litzsim"),b=dir.filePath("b.litzsim");
    Circuit::load(examples+"/rc.litzsim").save(a);
    Circuit::load(examples+"/buck.litzsim").save(b);
    auto* first=studio.open(a);
    first->edit([](Circuit& c){c.move("V1",12,3);});
    auto* second=studio.open(b);
    for (auto* item:second->scene()->items()) {
        auto* label=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
        if (label && label->text().startsWith("배선 경로 확인")) { qWarning()<<label->text();return 2; }
    }
    auto copy=second->circuit;
    copy.disconnectPin("C1.p");
    if (!copy.missingPins().contains("C1.p")) return 3;
    copy.connectPins("VC.p","C1.p");
    if (!copy.missingPins().isEmpty()) return 4;
    if (!second->history.isClean() || first->history.isClean() || second==first) return 1;
    if (studio.open(a)!=first || studio.current()!=first) return 1;
    first->save();
    if (Circuit::load(a).document!=first->circuit.document) return 1;
    first->history.undo();
    QEventLoop wait;
    QString output;
    studio.showResult=[&](const QString& path){output=path;wait.quit();};
    studio.startRun();
    auto snapshot=first->circuit;
    first->edit([](Circuit& c){c.move("C1",-7,19);});
    QTimer::singleShot(5000,&wait,&QEventLoop::quit);
    wait.exec();
    if (output.isEmpty() || Circuit::load(studio.runDirectory+"/circuit.litzsim").document!=snapshot.document) return 5;
    Waves result(output);
    qint64 row=0;
    simulate(snapshot,[&](double t,const QMap<QString,double>& values){
        if (result.at(row,0)!=t || result.at(row,1)!=values["VC_V"]) throw std::runtime_error("GUI/CLI mismatch");
        ++row;
    });
    if (row!=result.rows) return 6;
    first->edit([](Circuit& c){
        auto cfg=c.document["simulation"].toObject();cfg["stopTime_s"]=1000;c.document["simulation"]=cfg;
    });
    QEventLoop cancelWait;
    auto* runner=studio.findChild<QProcess*>();
    QObject::connect(runner,&QProcess::started,&studio,&Studio::stopRun);
    QObject::connect(runner,&QProcess::finished,&cancelWait,&QEventLoop::quit);
    studio.startRun();
    QTimer::singleShot(5000,&cancelWait,&QEventLoop::quit);cancelWait.exec();
    QFile report(studio.runDirectory+"/status.json");
    if(!report.open(QIODevice::ReadOnly) || QJsonDocument::fromJson(report.readAll()).object()["state"]!="canceled") return 7;
    return first->history.isClean() ? 1 : 0;
}
