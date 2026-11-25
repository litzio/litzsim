#include "solver.h"
#include <QJsonArray>
#include <cmath>
#include <cstdio>
#include <stdexcept>
#include <vector>

// A stiff 3 V output gives closed-form on/off current slopes. A one-cycle
// test distinguishes diode commutation from an inductor allowed to go negative.
int main(int argc,char** argv) {
    if (argc!=2) return 2;
    for (double initial:{0.2,0.0}) {
        auto c=Circuit::load(QString::fromLocal8Bit(argv[1]));
        auto parts=c.document["parts"].toObject();
        parts["C1"]=QJsonObject{{"type","dc"},{"parameters",QJsonObject{{"voltage_V",3}}}};
        parts["L1"]=QJsonObject{{"type","inductor"},{"parameters",QJsonObject{
            {"inductance_H",0.001},{"initialCurrent_A",initial}}}};
        c.document["parts"]=parts;
        constexpr double step=0.37e-6, end=100e-6, edge=50e-6;
        c.document["simulation"]=QJsonObject{{"timeStep_s",step},{"stopTime_s",end}};
        double peak=0,zero=end,maxError=0,priorTime=0,priorCurrent=initial,work=0,last=0;
        simulate(c,[&](double t,const QMap<QString,double>& v) {
            const double i=v["L1_A"];
            const double reference=t<=edge ? initial+2000*t :
                std::max(0.,initial+0.1-3000*(t-edge));
            maxError=std::max(maxError,std::abs(reference-i));
            peak=std::max(peak,i);
            if (t>edge && i<1e-10) zero=std::min(zero,t);
            if (v["D1_A"] < -1e-9) throw std::runtime_error("reverse rectifier current");
            work+=(t<=edge ? 2. : -3.)*(i+priorCurrent)*0.5*(t-priorTime);
            priorCurrent=last=i;
            priorTime=t;
        });
        double imbalance=std::abs(work-0.0005*(last*last-initial*initial));
        std::printf("initial_A=%.2f max_error_A=%g peak_A=%g zero_s=%g energy_error_J=%g\n",
                    initial,maxError,peak,zero,imbalance);
        if (maxError>1e-9 || std::abs(peak-initial-0.1)>1e-9 || imbalance>1e-9 ||
            (initial==0 && std::abs(zero-(edge+0.1/3000))>step))
            throw std::runtime_error("buck switching reference failed");
        auto settings=c.document["simulation"].toObject();
        std::vector<double> currents;
        for (int every:{1,10}) {
            settings["outputStep_s"]=every*step;
            c.document["simulation"]=settings;
            int row=0;
            simulate(c,[&](double t,const auto& values) {
                const double expected=std::min(end,(row*every)*step);
                if (std::abs(t-expected)>1e-15) throw std::runtime_error("output clock");
                if (every==1) currents.push_back(values["L1_A"]);
                else if (std::abs(values["L1_A"]-currents.at(t==end ? currents.size()-1 : row*every))>1e-12)
                    throw std::runtime_error("recording stride changed trajectory");
                ++row;
            });
            if (row!=1+int(std::ceil(end/(every*step)))) throw std::runtime_error("missing final sample");
        }
        // A second rising edge commutates a still-conducting freewheel diode in CCM.
        settings.remove("outputStep_s");
        settings["stopTime_s"]=2*end;
        c.document["simulation"]=settings;
        double lastAccepted=0;
        try {
            simulate(c,[&](double t,const auto& values){
                lastAccepted=t;
                if(t<=end) return;
                const double phase=t-end;
                const double start=std::max(0.,initial-0.05);
                const double expected=phase<=edge ? start+2000*phase :
                    std::max(0.,start+0.1-3000*(phase-edge));
                if(std::abs(values["L1_A"]-expected)>1e-9 ||
                   (phase<=edge && std::abs(values["D1_A"])>1e-9))
                    throw std::runtime_error("incorrect second-cycle commutation");
            });
        } catch(const std::exception& e) {
            std::fprintf(stderr,"after %.12g s: %s\n",lastAccepted,e.what());
            return 1;
        }
    }
}
