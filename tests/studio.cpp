#include "studio.h"
#include "waves.h"
#include "solver.h"
#include <QEventLoop>
#include <QTimer>
#include <QJsonDocument>
#include <QApplication>
#include <QTemporaryDir>
#include <QGraphicsSimpleTextItem>
int main(int argc,char** argv) {
    QApplication app(argc,argv);
    if (argc!=2) return 2;
    QTemporaryDir dir;
    Studio studio(dir.path());
    QString examples=QString::fromLocal8Bit(argv[1]);
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
