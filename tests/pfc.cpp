#include "solver.h"
#include <QElapsedTimer>
#include <QJsonArray>
#include <array>
#include <cmath>
#include <cstdio>
#include <stdexcept>
#include <vector>

struct Report {
    std::vector<std::array<double,3>> means;
    double peak=0,minimum=1e5,energyError=0;
};
Report run(Circuit c,QString mode) {
    auto cfg=c.document["simulation"].toObject();cfg["model"]=mode;c.document["simulation"]=cfg;
    Report out;
    std::array<double,3> sum{},previous{};
    int row=0;
    QElapsedTimer timer;timer.start();
    const auto parts=c.document["parts"].toObject();
    auto parameter=[&](QString name,QString key){return parts[name].toObject()["parameters"].toObject()[key].toDouble();};
    const double cap=parameter("Cdc","capacitance_F"),v0=parameter("Cdc","initialVoltage_V");
    simulate(c,[&](double,const QMap<QString,double>& values){
        std::array<double,3> current{values.value("IL1_A"),values.value("IL2_A"),values.value("Vdc_V")};
        out.peak=std::max(out.peak,std::abs(current[0]+current[1]));out.minimum=std::min(out.minimum,current[2]);
        double energy=.5*cap*(current[2]*current[2]-v0*v0);
        for(int j=0;j<2;++j) energy+=.5*parameter(j ? "L2":"L1","inductance_H")*current[j]*current[j];
        out.energyError=std::max(out.energyError,std::abs(energy-values.value("gridEnergy_J")+values.value("loadEnergy_J")));
        if(row>0) {
            for(int j=0;j<3;++j) sum[j]+=(current[j]+previous[j])/32;
            if(row%16==0) {out.means.push_back(sum);sum={};}
        }
        previous=current;++row;
    });
    std::printf("%s periods=%zu peak_grid_A=%.4f min_bus_V=%.4f max_energy_error_J=%.9g validation_ms=%lld\n",
        qPrintable(mode),out.means.size(),out.peak,out.minimum,out.energyError,static_cast<long long>(timer.elapsed()));
    return out;
}
int main(int argc,char** argv) {
    if(argc!=2) return 2;
    auto c=Circuit::load(QString::fromLocal8Bit(argv[1]));
    auto sw=run(c,"pfc-switching"),avg=run(c,"pfc-average");
    if(sw.means.size()!=25000 || sw.means.size()!=avg.means.size() ||
       sw.energyError>1e-5 || avg.energyError>1e-5) return 1;
    std::array<double,3> error{};
    for(size_t k=0;k<sw.means.size();++k) for(int j=0;j<3;++j)
        error[j]+=std::pow(sw.means[k][j]-avg.means[k][j],2);
    for(auto& e:error)e=std::sqrt(e/sw.means.size());
    std::printf("period mean RMSE: %.5f A, %.5f A, %.5f V\n",error[0],error[1],error[2]);
    if(error[0]>1 || error[1]>1 || error[2]>.5) return 1;
    for(QString mode:{"pfc-average","pfc-switching"}) {
        auto timed=c;
        auto cfg=timed.document["simulation"].toObject();
        cfg["model"]=mode;cfg["stopTime_s"]=80e-6;
        std::vector<double> full;
        for(int stride:{1,7}) {
            cfg["outputStep_s"]=stride*1e-6;timed.document["simulation"]=cfg;
            int row=0;
            simulate(timed,[&](double t,const auto& values){
                const int index=std::min(row*stride,80);
                if(std::abs(t-index*1e-6)>1e-15) throw std::runtime_error("PFC output clock");
                if(stride==1) full.push_back(values["Vdc_V"]);
                else if(values["Vdc_V"]!=full.at(index)) throw std::runtime_error("PFC recording changed control");
                ++row;
            });
            if(row!=(stride==1 ? 81 : 13)) return 4;
        }
    }
    c.disconnectPin("Q1H.d");c.connectPins("Q1H.d","Q1L.s");
    try {simulate(c,[](double,const auto&){});} catch(const std::exception&) {return 0;}
    return 3;
}
