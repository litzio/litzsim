#include "viewer.h"
#include <QApplication>
#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QFileDialog>
#include <QFormLayout>
#include <QLabel>
#include <QLineEdit>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPainter>
#include <QProcess>
#include <QStatusBar>
#include <QUuid>
#include <QWheelEvent>
#include <algorithm>
#include <cmath>
#include <limits>

namespace {
const QColor colors[]={QColor("#624585"),QColor("#12786f"),QColor("#c66421"),QColor("#3077ae")};
void extend(double& lo,double& hi,double value) {lo=std::min(lo,value);hi=std::max(hi,value);}
void apply(Scale& axis,double low,double high) {
    if (!axis.automatic || !std::isfinite(low) || !std::isfinite(high)) return;
    double pad=high==low ? std::max(1e-9,std::abs(low)*.05) : 0;
    axis.low=low-pad;axis.high=high+pad;
}
void dialogButtons(QDialog& dialog,QFormLayout& form) {
    auto* buttons=new QDialogButtonBox(QDialogButtonBox::Ok|QDialogButtonBox::Cancel,&dialog);
    form.addRow(buttons);
    QObject::connect(buttons,&QDialogButtonBox::accepted,&dialog,&QDialog::accept);
    QObject::connect(buttons,&QDialogButtonBox::rejected,&dialog,&QDialog::reject);
}
}
Plot::Plot(QWidget* parent):QWidget(parent) {
    setMinimumSize(300,125);setSizePolicy(QSizePolicy::Expanding,QSizePolicy::Expanding);
}
QRect Plot::graph() const {return rect().adjusted(88,24,-88,-29);}
void Plot::fit() {x.automatic=left.automatic=right.automatic=true;update();}
void Plot::paintEvent(QPaintEvent*) {
    QPainter p(this);p.fillRect(rect(),Qt::white);
    QRect area=graph();if(area.width()<1 || area.height()<1) return;
    double xmin=INFINITY,xmax=-INFINITY;
    for (auto& s:signalsList) {
        if (s.x==0) {
            extend(xmin,xmax,s.data->at(0,0));extend(xmin,xmax,s.data->at(s.data->rows-1,0));
        } else if (x.automatic) {
            for (auto pt:s.data->envelope(s.data->at(0,0),s.data->at(s.data->rows-1,0),s.x,area.width()))
                extend(xmin,xmax,pt.y());
        }
    }
    apply(x,xmin,xmax);
    double lows[]={INFINITY,INFINITY},highs[]={-INFINITY,-INFINITY};
    std::vector<std::vector<QPointF>> lines;
    for (auto& s:signalsList) {
        auto& points=lines.emplace_back();
        if (s.x==0) points=s.data->envelope(x.low,x.high,s.y,area.width());
        else {
            // Merge X and Y extrema by acquisition index so a reversal of X never reorders samples.
            const double t0=s.data->at(0,0),t1=s.data->at(s.data->rows-1,0);
            auto xr=s.data->envelopeRows(t0,t1,s.x,area.width()*2);
            auto yr=s.data->envelopeRows(t0,t1,s.y,area.width()*2);
            std::vector<qint64> rows;
            std::set_union(xr.begin(),xr.end(),yr.begin(),yr.end(),std::back_inserter(rows));
            for(auto i:rows) points.emplace_back(s.data->at(i,s.x),s.data->at(i,s.y));
        }
        for (auto pt:points) extend(lows[s.right],highs[s.right],pt.y());
    }
    apply(left,lows[0],highs[0]);apply(right,lows[1],highs[1]);
    p.setPen(QPen(QColor("#e2e4e9"),1));
    for (int i=0;i<=4;++i) {
        int px=area.left()+i*area.width()/4,py=area.bottom()-i*area.height()/4;
        p.setPen(QColor("#e2e4e9"));p.drawLine(px,area.top(),px,area.bottom());p.drawLine(area.left(),py,area.right(),py);
        p.setPen(QColor("#434955"));
        p.drawText(QRect(px-42,area.bottom()+2,84,18),Qt::AlignCenter,QString::number(x.low+(x.high-x.low)*i/4,'g',4));
        p.drawText(QRect(1,py-9,area.left()-7,18),Qt::AlignRight,QString::number(left.low+(left.high-left.low)*i/4,'g',4));
        if (std::isfinite(lows[1])) p.drawText(QRect(area.right()+6,py-9,60,18),Qt::AlignLeft,
            QString::number(right.low+(right.high-right.low)*i/4,'g',4));
    }
    int legend=area.left();
    for (size_t i=0;i<lines.size();++i) {
        auto& s=signalsList[i];auto axis=s.right ? right : left;
        const QString name=QString("%1 · %2 / %3%4").arg(s.source,s.data->channels[s.y],s.data->channels[s.x],s.right?" (R)":"");
        p.setPen(colors[i%4]);p.drawText(legend,17,name);legend+=p.fontMetrics().horizontalAdvance(name)+18;
        QPolygonF pixels;pixels.reserve(lines[i].size());
        for (auto pt:lines[i]) pixels.append(QPointF(area.left()+(pt.x()-x.low)/(x.high-x.low)*area.width(),
            area.bottom()-(pt.y()-axis.low)/(axis.high-axis.low)*area.height()));
        p.save();p.setClipRect(area);p.setPen(QPen(colors[i%4],0));p.drawPolyline(pixels);p.restore();
    }
}
void Plot::wheelEvent(QWheelEvent* e) {
    if (selected) selected(this);
    const double ratio=e->angleDelta().y()>0 ? 0.8 : 1.25;
    double fraction=(e->position().x()-graph().left())/graph().width();
    auto zoom=[&](Scale& axis,double f){
        const double at=axis.low+f*(axis.high-axis.low);
        axis.low=at+(axis.low-at)*ratio;axis.high=at+(axis.high-at)*ratio;axis.automatic=false;
    };
    if (e->modifiers()&Qt::ShiftModifier) {
        double f=(graph().bottom()-e->position().y())/graph().height();zoom(left,f);zoom(right,f);
    } else zoom(x,fraction);
    if (changed) changed(this);update();e->accept();
}
void Plot::mousePressEvent(QMouseEvent* e) {drag=e->pos();if(selected) selected(this);}
void Plot::mouseMoveEvent(QMouseEvent* e) {
    if (!(e->buttons()&Qt::LeftButton)) return;
    auto diff=e->pos()-drag;drag=e->pos();
    auto shift=[](Scale& s,double amount){double delta=amount*(s.high-s.low);s.low+=delta;s.high+=delta;s.automatic=false;};
    shift(x,-double(diff.x())/graph().width());
    if (diff.y()) {shift(left,double(diff.y())/graph().height());shift(right,double(diff.y())/graph().height());}
    if(changed) changed(this);update();
}
void Plot::mouseDoubleClickEvent(QMouseEvent*) {fit();if(changed) changed(this);}
Viewer::Viewer(QWidget* parent):QMainWindow(parent,Qt::Window) {
    setWindowTitle("LitzSim · 파형");resize(1100,720);setAttribute(Qt::WA_DeleteOnClose);
    auto* body=new QWidget;plots=new QVBoxLayout(body);setCentralWidget(body);
    auto* bar=addToolBar("파형");
    bar->addAction("결과 열기",this,[this]{
        auto path=QFileDialog::getOpenFileName(this,"파형 열기",{},"파형 (*.wave *.csv)");
        if (!path.isEmpty()) open(path);
    });
    bar->addAction("Plot 추가",this,[this]{addPlot();});
    bar->addAction("Plot 삭제",this,[this]{
        if (!active) return;
        auto* old=active;plots->removeWidget(old);old->deleteLater();
        active=plots->count() ? static_cast<Plot*>(plots->itemAt(0)->widget()) : addPlot();
    });
    bar->addAction("신호 겹치기",this,[this]{chooseSignal();});
    bar->addAction("축 설정",this,[this]{ranges();});
    bar->addAction("전체 보기",this,[this]{if(active){active->fit();active->changed(active);}});
    link=bar->addAction("시간축 연결");link->setCheckable(true);
    addPlot();
}
Plot* Viewer::addPlot() {
    auto* plot=new Plot;plots->addWidget(plot);active=plot;
    plot->selected=[this](Plot* p){active=p;statusBar()->showMessage(QString("Plot %1").arg(plots->indexOf(p)+1));};
    plot->changed=[this](Plot* p){
        auto isTime=[](Plot* v){return std::all_of(v->signalsList.begin(),v->signalsList.end(),[](auto& s){return s.x==0;});};
        if (!link->isChecked() || !isTime(p)) return;
        for (int i=0;i<plots->count();++i) {
            auto* other=static_cast<Plot*>(plots->itemAt(i)->widget());
            if (other!=p && isTime(other)) {other->x=p->x;other->update();}
        }
    };
    return plot;
}
void Viewer::open(const QString& path) {
    const QString label=QFileInfo(path).completeBaseName()=="result" ?
        QFileInfo(path).dir().dirName().left(8) : QFileInfo(path).completeBaseName();
    if (!path.endsWith(".csv",Qt::CaseInsensitive)) {addSource(path,label);return;}
    QString target=cache.filePath(QUuid::createUuid().toString()+".wave");
    auto* job=new QProcess(this);
    connect(job,&QProcess::finished,this,[this,job,target,label](int code,QProcess::ExitStatus status){
        if (!code && status==QProcess::NormalExit) addSource(target,label);
        else statusBar()->showMessage(QString::fromUtf8(job->readAllStandardError()));
        job->deleteLater();
    });
    connect(job,&QProcess::errorOccurred,this,[this,job](auto){statusBar()->showMessage(job->errorString());});
    job->start(QCoreApplication::applicationDirPath()+"/litzsim-cli.exe",{"import-csv",path,target});
    statusBar()->showMessage("CSV 읽는 중 · "+label);
}
void Viewer::addSource(const QString& path,const QString& label) {
    try {
        auto data=std::make_shared<Waves>(path);
        files.emplace_back(label,data);
        if (data->channels.size()>1) active->signalsList.push_back({data,label,0,1,false});
        active->fit();statusBar()->showMessage(QString("%1 · %2 samples").arg(label).arg(data->rows));
    } catch(const std::exception& e){statusBar()->showMessage(QString::fromUtf8(e.what()));}
}
void Viewer::chooseSignal() {
    if(files.empty() || !active) return;
    QDialog dialog(this);dialog.setWindowTitle("신호 겹치기");QFormLayout form(&dialog);
    QComboBox source,xc,yc;QCheckBox r("오른쪽 Y축");
    for (auto& f:files) source.addItem(f.first);
    auto populate=[&](int i){xc.clear();yc.clear();xc.addItems(files[i].second->channels);yc.addItems(files[i].second->channels);yc.setCurrentIndex(std::min(1,yc.count()-1));};
    connect(&source,&QComboBox::currentIndexChanged,&dialog,populate);populate(0);
    form.addRow("실행", &source);form.addRow("X",&xc);form.addRow("Y",&yc);form.addRow(&r);
    dialogButtons(dialog,form);
    if (dialog.exec()==QDialog::Accepted) {
        auto& [label,data]=files[source.currentIndex()];
        active->signalsList.push_back({data,label,xc.currentIndex(),yc.currentIndex(),r.isChecked()});active->fit();
    }
}
void Viewer::ranges() {
    if (!active) return;
    QDialog dialog(this);dialog.setWindowTitle("축 범위");QFormLayout form(&dialog);
    Scale* axes[]={&active->x,&active->left,&active->right};
    QCheckBox automatic[3];QLineEdit low[3],high[3];
    QStringList labels={"X","Y (L)","Y (R)"};
    for (int i=0;i<3;++i) {
        automatic[i].setChecked(axes[i]->automatic);automatic[i].setText(labels[i]+" 자동");
        low[i].setText(QString::number(axes[i]->low,'g',15));high[i].setText(QString::number(axes[i]->high,'g',15));
        form.addRow(&automatic[i]);form.addRow("최소",&low[i]);form.addRow("최대",&high[i]);
    }
    dialogButtons(dialog,form);
    if (dialog.exec()!=QDialog::Accepted) return;
    Scale pending[3];
    for (int i=0;i<3;++i) {
        bool a,b;pending[i]={low[i].text().toDouble(&a),high[i].text().toDouble(&b),automatic[i].isChecked()};
        if (!pending[i].automatic && (!a || !b || !(pending[i].low<pending[i].high))) {
            statusBar()->showMessage("축 범위를 확인하세요.");return;
        }
    }
    for (int i=0;i<3;++i) *axes[i]=pending[i];
    active->update();active->changed(active);
}
