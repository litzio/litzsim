#include "studio.h"
#include "viewer.h"
#include <QApplication>
#include <QEventLoop>
#include <QGraphicsSimpleTextItem>
#include <QTemporaryDir>
#include <QTimer>
#include <QMouseEvent>
#include <QScrollBar>
#include <cstdio>
int main(int argc,char** argv) {
    QApplication app(argc,argv);
    if(argc<2) return 2;
    QTemporaryDir dir;
    const auto path=dir.filePath("pfc.litzsim");
    auto initial=Circuit::load(QString::fromLocal8Bit(argv[1]));
    initial.save(path);
    Studio studio(dir.path());
    auto* sheet=studio.open(path);
    studio.resize(1600,900);studio.show();app.processEvents();
    auto bounds=sheet->scene()->itemsBoundingRect().adjusted(-38,-45,38,45);
    sheet->fitInView(bounds,Qt::KeepAspectRatio);sheet->centerOn(bounds.center());
    // Select Q1H via its name as a user would.
    for(auto* item:sheet->scene()->items()) {
        auto* label=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
        if (label && label->text().startsWith("배선 경로 확인")) {qWarning()<<label->text();return 3;}
        if (!label || label->text()!="Q1H") continue;
        auto point=sheet->mapFromScene(label->sceneBoundingRect().center());
        auto global=sheet->viewport()->mapToGlobal(point);
        QMouseEvent press(QEvent::MouseButtonPress,point,global,Qt::LeftButton,Qt::LeftButton,Qt::NoModifier);
        QApplication::sendEvent(sheet->viewport(),&press);
        QMouseEvent release(QEvent::MouseButtonRelease,point,global,Qt::LeftButton,Qt::NoButton,Qt::NoModifier);
        QApplication::sendEvent(sheet->viewport(),&release);break;
    }
    if(argc>2 && !studio.grab().save(QString::fromLocal8Bit(argv[2])+"/pfc-editor.png")) return 4;
    Viewer viewer;
    std::vector<Signal> signalsOut;
    for(QString mode:{"pfc-switching","pfc-average"}) {
        sheet->edit([&](Circuit& c){
            auto cfg=c.document["simulation"].toObject();cfg["model"]=mode;c.document["simulation"]=cfg;
            c.setParameter("L1","inductance_H",410e-6);
        });
        QEventLoop loop;QString pathOut;
        studio.showResult=[&](const QString& path){pathOut=path;loop.quit();};
        studio.startRun();QTimer::singleShot(30000,&loop,&QEventLoop::quit);loop.exec();
        if(pathOut.isEmpty()) return 5;
        if(Circuit::load(studio.runDirectory+"/circuit.litzsim").document!=sheet->circuit.document) return 6;
        viewer.open(pathOut);auto signal=viewer.active->signalsList.back();signal.source=mode;
        signal.y=signal.data->channels.indexOf("Vdc_V");signalsOut.push_back(signal);
        if(signal.data->rows!=400001) return 7;
    }
    viewer.active->signalsList=signalsOut;
    auto* amps=viewer.addPlot();for(auto s:signalsOut){s.y=s.data->channels.indexOf("IL1_A");amps->signalsList.push_back(s);}
    viewer.resize(1280,720);viewer.show();app.processEvents();
    if(argc>2 && !viewer.grab().save(QString::fromLocal8Bit(argv[2])+"/pfc-waveforms.png")) return 8;
    return 0;
}
