#include "studio.h"
#include "viewer.h"
#include "solver.h"
#include <QApplication>
#include <QElapsedTimer>
#include <QFontDatabase>
#include <QFontInfo>
#include <QImage>
#include <algorithm>
#include <cmath>
#include <QTemporaryDir>
#include <cstdio>
int main(int argc,char** argv) {
    QApplication app(argc,argv);
    if (argc<2) return 2;
    const QString examples=QString::fromLocal8Bit(argv[1]);
    const bool capture=argc>2;
    QTemporaryDir dir;
    qInfo()<<"platform"<<app.platformName()<<"initial font"<<QFontInfo(app.font()).family()
           <<"available families"<<QFontDatabase::families().size();
    std::printf("platform=%s system_family=%s available_families=%lld\n",qPrintable(app.platformName()),
                qPrintable(QFontInfo(app.font()).family()),static_cast<long long>(QFontDatabase::families().size()));
    Studio studio(examples);
    qInfo()<<"product font"<<QFontInfo(app.font()).family()<<QFontInfo(app.font()).pixelSize();
    std::printf("app_family=%s pixels=%d\n",qPrintable(QFontInfo(app.font()).family()),QFontInfo(app.font()).pixelSize());
    studio.open(examples+"/rc.litzsim");
    auto* sheet=studio.open(examples+"/buck.litzsim");
    studio.resize(1280,720);studio.show();app.processEvents();
    sheet->fitInView(sheet->scene()->itemsBoundingRect().adjusted(-50,-50,50,50),Qt::KeepAspectRatio);
    app.processEvents();
    if (capture && !studio.grab().save(QString::fromLocal8Bit(argv[2])+"/editor.png")) return 2;
    Viewer viewer;
    auto buck=dir.filePath("Buck.wave");
    simulateWave(sheet->circuit,buck);
    viewer.open(buck);
    auto source=viewer.active->signalsList.front();
    source.y=source.data->channels.indexOf("VC_V");
    viewer.active->signalsList={source};
    auto* current=viewer.addPlot();source.y=source.data->channels.indexOf("L1_A");current->signalsList={source};
    viewer.resize(1100,680);viewer.show();app.processEvents();
    if (capture && !viewer.grab().save(QString::fromLocal8Bit(argv[2])+"/waveforms.png")) return 3;
    if (source.data->rows<=1000 || viewer.active!=current) return 1;
    if (argc>3) {
        const auto bench=dir.filePath("bench.wave");
        WaveWriter writer(bench,{"time_s","a","b","c","d","e","f","g","h"});
        std::vector<double> values(9);
        for (int i=0;i<10000000;++i) {
            values[0]=i*1e-6;
            for(int j=1;j<9;++j) values[j]=(i+j*7)%(143+j*19);
            writer.append(values);
        }
        writer.finish();
        Viewer four;
        four.resize(1280,780);
        QElapsedTimer timer;timer.start();
        four.open(bench);
        auto signal=four.active->signalsList.front();
        QList<Plot*> all{four.active};
        for (int i=1;i<4;++i) {
            auto* panel=four.addPlot();signal.y=i+1;panel->signalsList.push_back(signal);all<<panel;
        }
        four.show();app.processEvents();
        QImage frame(four.size(),QImage::Format_RGB32);
        four.render(&frame);
        auto first=timer.elapsed();
        std::vector<double> times;
        for(int i=0;i<50;++i) {
            for(auto* panel:all) panel->x={i*0.01,9+i*0.01,false};
            timer.restart();four.render(&frame);times.push_back(timer.nsecsElapsed()/1e6);
        }
        std::sort(times.begin(),times.end());
        std::printf("first_render_ms=%lld four_plot_p95_ms=%.2f\n",static_cast<long long>(first),times[47]);
    }
    return 0;
}
