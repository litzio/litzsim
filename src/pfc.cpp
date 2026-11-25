#include "pfc.h"
#include <QJsonArray>
#include <QSet>
#include <algorithm>
#include <array>
#include <cmath>
#include <stdexcept>

namespace {
constexpr double pi=3.14159265358979323846;
void check(bool ok,const QString& message) {
    if(!ok) throw std::runtime_error(message.toStdString());
}
// Compile the supported two-phase charging topology before reducing it to
// iL1, iL2, and Vdc. An edit that changes the topology cannot silently reuse it.
struct Plant {
    double amplitude,omega,inductance[2],capacitance,load,current[2],voltage;
    QString sensors[2],bus;
    double workIn=0,workLoad=0;
};
Plant compile(const Circuit& c) {
    auto parts=c.document["parts"].toObject();
    auto ctrl=c.document["control"].toObject();
    QSet<QString> used;
    auto role=[&](QString key,QString type){
        QString id=ctrl["parts"].toObject()[key].toString();
        check(!used.contains(id) && parts[id].toObject()["type"]==type,"invalid PFC role: "+key);
        used.insert(id);return id;
    };
    auto value=[&](QString id,QString key){return parts[id].toObject()["parameters"].toObject()[key].toDouble();};
    QMap<QString,QString> net;
    auto nets=c.document["nets"].toObject();
    for(auto it=nets.begin();it!=nets.end();++it)
        for(auto pin:it.value().toArray()) net[pin.toString()]=it.key();
    auto same=[&](QString a,QString b){
        check(net.contains(a) && net.contains(b) && net[a]==net[b],"unsupported PFC connection: "+a+" / "+b);
    };
    Plant p{};
    auto ac=role("grid","sine"),cap=role("link","capacitor"),sink=role("load","sink");
    p.bus=role("voltageProbe","voltmeter");
    auto ground=role("reference","ground");
    same(cap+".p",sink+".p");same(cap+".n",sink+".n");same(cap+".n",ground+".ref");
    same(cap+".p",p.bus+".p");same(cap+".n",p.bus+".n");
    p.amplitude=value(ac,"amplitude_V");p.omega=2*pi*value(ac,"frequency_Hz");
    p.capacitance=value(cap,"capacitance_F");p.voltage=value(cap,"initialVoltage_V");
    p.load=value(sink,"current_A");
    auto gates=ctrl["gateSignals"].toObject();
    QSet<QString> powerNodes{net[cap+".p"],net[cap+".n"],net[ac+".p"],net[ac+".n"]};
    for(int j=0;j<3;++j) {
        QString phase=j==2 ? "N" : QString::number(j+1);
        QString high=role("high"+phase,"nmos"),low=role("low"+phase,"nmos");
        check(gates["high"+phase]==high+".g" && gates["low"+phase]==low+".g","PFC gate mapping: "+phase);
        same(high+".d",cap+".p");same(low+".s",cap+".n");same(high+".s",low+".d");
        if(j==2) same(high+".s",ac+".n");
        else {
            auto l=role("inductor"+phase,"inductor");
            p.sensors[j]=role("sensor"+phase,"ammeter");
            same(l+".p",ac+".p");same(l+".n",p.sensors[j]+".p");same(p.sensors[j]+".n",high+".s");
            powerNodes.insert(net[l+".n"]);powerNodes.insert(net[high+".s"]);
            p.inductance[j]=value(l,"inductance_H");p.current[j]=value(l,"initialCurrent_A");
        }
    }
    check(powerNodes.size()==8 && used.size()==parts.size() && gates.size()==6,"unsupported short or additional PFC device");
    check(p.inductance[0]>0 && p.inductance[1]>0 && p.capacitance>0 && p.voltage>0 && p.omega>0,
          "invalid PFC plant parameters");
    return p;
}
struct Controller {
    double z[11]={-28*std::sqrt(2.)};
    double duty[2]={0.5,0.5};
    int neutral=0;
    void update(const Plant& p,double t,double period,double reference) {
        double dz[11]={};
        const double angle=1.5*pi+p.omega*t;
        const double cs=std::cos(angle),sn=std::sin(angle);
        const double grid=p.amplitude*std::sin(p.omega*t);
        const double ev=reference-p.voltage;
        double raw=.664*ev+z[0],limit=std::clamp(raw,-500.,500.);
        dz[0]=8.8449*(ev+(limit-raw)/.1328);
        const double measurements[]={grid,p.current[0],p.current[1]};
        for(int k=0;k<3;++k) {
            int a=5+2*k;
            dz[a]=p.omega*(measurements[k]-z[a]-z[a+1]);dz[a+1]=p.omega*z[a];
        }
        double modulation[2];
        for(int j=0;j<2;++j) {
            const double i=p.current[j],beta=z[8+2*j];
            const double ed=.5*limit-(i*cs+beta*sn),eq=i*sn-beta*cs;
            const int d=1+2*j;
            dz[d]=500*ed;dz[d+1]=500*eq;
            modulation[j]=(grid-(6*ed+z[d])*cs+(6*eq+z[d+1])*sn)/p.voltage;
        }
        neutral=modulation[0]<0;
        for(int j=0;j<2;++j) duty[j]=std::clamp(modulation[j]+neutral,.01,.99);
        for(int i=0;i<11;++i) z[i]+=period*dz[i];
    }
};
void advance(Plant& p,double t,double h,double m1,double m2) {
    double vg=p.amplitude*std::sin(p.omega*(t+h/2));
    double a=h/(2*p.inductance[0]),b=h/(2*p.inductance[1]),k=h/(2*p.capacitance);
    double vm=(p.voltage+k*(m1*p.current[0]+m2*p.current[1]-p.load+(a*m1+b*m2)*vg))/
              (1+k*(a*m1*m1+b*m2*m2));
    double di1=a*(vg-m1*vm),di2=b*(vg-m2*vm);
    p.workIn+=h*vg*(p.current[0]+di1+p.current[1]+di2);p.workLoad+=h*vm*p.load;
    p.current[0]+=2*di1;p.current[1]+=2*di2;p.voltage=2*vm-p.voltage;
    check(std::isfinite(p.voltage) && p.voltage>0,"DC link collapsed");
}
}
void simulatePfc(const Circuit& circuit,const Sample& sample,bool average) {
    Plant p=compile(circuit);
    auto cfg=circuit.document["simulation"].toObject(),ctrl=circuit.document["control"].toObject();
    const double step=cfg["timeStep_s"].toDouble(),end=cfg["stopTime_s"].toDouble();
    const double frequency=ctrl["pwm_Hz"].toDouble(),controlPeriod=ctrl["sample_s"].toDouble();
    const double reference=ctrl["linkReference_V"].toDouble();
    check(frequency>0 && controlPeriod>0 && reference>0,"invalid PFC timing or reference");
    // This spike uses an integer output grid; PWM crossings inside an interval
    // are solved exactly on each carrier slope. Controller ticks retain their own clock.
    const double halfRatio=.5/(frequency*step),sampleRatio=controlPeriod/step;
    auto ticks=[](double ratio) {
        check(std::isfinite(ratio) && ratio>=1-1e-8 && ratio<=1e10 &&
              std::abs(ratio-std::round(ratio))<1e-7,"PFC clocks must be multiples of time-step");
        return static_cast<qint64>(std::llround(ratio));
    };
    auto half=ticks(halfRatio),controlTicks=ticks(sampleRatio),steps=ticks(end/step);
    Controller ctl;
    const qint64 stride=std::max<qint64>(1,circuit.outputStride());
    QMap<QString,double> values;
    for(qint64 n=0;n<=steps;++n) {
        double t=n*step;
        values[p.sensors[0]+"_A"]=p.current[0];values[p.sensors[1]+"_A"]=p.current[1];
        values[p.bus+"_V"]=p.voltage;values["gridEnergy_J"]=p.workIn;values["loadEnergy_J"]=p.workLoad;
        if(n%stride==0 || n==steps) sample(t,values);
        if(n==steps) break;
        if(n%controlTicks==0) ctl.update(p,t,controlPeriod,reference);
        if(average) {advance(p,t,step,ctl.duty[0]-ctl.neutral,ctl.duty[1]-ctl.neutral);continue;}
        const auto phase=n%(2*half);
        const double carrier=phase<=half ? double(phase)/half : 2.-double(phase)/half;
        const double slope=(phase<half?1.:-1.)/half;
        std::array<double,4> edges={0,std::clamp((ctl.duty[0]-carrier)/slope,0.,1.),
            std::clamp((1-ctl.duty[1]-carrier)/slope,0.,1.),1};
        std::sort(edges.begin(),edges.end());
        for(int i=1;i<4;++i) {
            if(edges[i]==edges[i-1]) continue;
            const double triangle=carrier+(edges[i]+edges[i-1])*.5*slope;
            advance(p,t+edges[i-1]*step,(edges[i]-edges[i-1])*step,
                int(triangle<ctl.duty[0])-ctl.neutral,int(1-triangle<ctl.duty[1])-ctl.neutral);
        }
    }
}
