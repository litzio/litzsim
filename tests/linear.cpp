#include "solver.h"
#include <cmath>
#include <cstdio>
#include <stdexcept>
int main(int argc,char** argv) {
    if (argc!=2) return 2;
    for (QString name:{"rc","rlc"}) {
        auto c=Circuit::load(QString::fromLocal8Bit(argv[1])+"/"+name+".litzsim");
        double previous=1e6;
        for (double h:{1e-5,5e-6,2.5e-6}) {
            auto settings=c.document["simulation"].toObject();
            settings["timeStep_s"]=h;
            c.document["simulation"]=settings;
            double error=0;
            simulate(c,[&](double t,const QMap<QString,double>& values){
                // Unit step multiplied by 5 V. Series RLC: alpha=500, wd=sqrt(750000).
                const double wd=std::sqrt(750000.);
                double ref=name=="rc" ? 5*(1-std::exp(-1000*t)) :
                    5*(1-std::exp(-500*t)*(std::cos(wd*t)+500/wd*std::sin(wd*t)));
                error=std::max(error,std::abs(values.value("VC_V",1e10)-ref));
            });
            std::printf("%s h=%.3g maxError_V=%.8g\n",qPrintable(name),h,error);
            if (error>0.04 || error>previous*0.55) throw std::runtime_error("accuracy or convergence limit");
            previous=error;
        }
    }
}
