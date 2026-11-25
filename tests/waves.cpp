#include "waves.h"
#include <QTemporaryDir>
#include <QElapsedTimer>
#include <cstdio>
#include <stdexcept>
int main(int argc,char**) {
    const int count=argc>1 ? 10000000 : 1001;
    QTemporaryDir dir;
    QStringList names={"time_s","a","b","c","d","e","f","g","h"};
    auto csv=dir.filePath("samples.csv"),wave=dir.filePath("samples.wave");
    QFile file(csv);
    if (!file.open(QIODevice::WriteOnly)) return 1;
    file.write(names.join(',').toUtf8()+"\n");
    std::vector<double> values(9);
    WaveWriter native(dir.filePath("native.wave"),names);
    for (int i=0;i<count;++i) {
        const bool peak=i==256;
        values[0]=i;
        values[1]=peak ? 23 : 0;
        native.append(values);
        file.write(QByteArray::number(i)+(peak ? ",23" : ",0")+",0,0,0,0,0,0,0\n");
    }
    native.finish();
    file.close();
    QElapsedTimer timer;
    timer.start();
    importCsv(csv,wave);
    std::printf("CSV bytes=%lld import_ms=%lld\n",static_cast<long long>(file.size()),static_cast<long long>(timer.restart()));
    for (const QString& path:{wave,dir.filePath("native.wave")}) {
        timer.restart();
        Waves result(path);
        auto points=result.envelope(0,count-1,1,1000);
        bool found=false;
        for (auto point:points) if (point.x()==256 && point.y()==23) found=true;
        if (!found || result.rows!=count || result.at(255,1)!=0 || result.at(257,1)!=0)
            throw std::runtime_error("lost peak or sample");
        std::printf("rows=%d open_and_query_ms=%lld points=%zu\n",count,static_cast<long long>(timer.elapsed()),points.size());
    }
}
