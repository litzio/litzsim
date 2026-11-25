#include "waves.h"
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSet>
#include <algorithm>
#include <charconv>
#include <cmath>
#include <cstring>
#include <stdexcept>
#include <limits>

namespace {
constexpr int block=256;
constexpr qint64 header=4096;
void ensure(bool ok,const char* msg) { if (!ok) throw std::runtime_error(msg); }
void validateChannels(const QStringList& names) {
    QSet<QString> unique(names.begin(),names.end());
    ensure(!names.empty() && names.size()<=128 && !unique.contains("") && unique.size()==names.size(),"invalid channel names");
}
}
WaveWriter::WaveWriter(const QString& path,const QStringList& channels):file(path),columns(channels.size()),
    minimum(columns),maximum(columns),offsets(2*columns) {
    static_assert(Q_BYTE_ORDER==Q_LITTLE_ENDIAN && std::numeric_limits<double>::is_iec559);
    validateChannels(channels);
    QByteArray names=QJsonDocument(QJsonArray::fromStringList(channels)).toJson(QJsonDocument::Compact);
    ensure(names.size()<header-16,"channel header too long");
    QByteArray bytes(header,0);
    std::memcpy(bytes.data(),"LITZWV01",8);
    std::memcpy(bytes.data()+16,names.data(),names.size());
    ensure(file.open(QIODevice::WriteOnly) && file.write(bytes)==header,"cannot create wave file");
}
void WaveWriter::append(const std::vector<double>& row) {
    ensure(int(row.size())==columns,"wrong sample width");
    ensure(count==0 || row.front()>=lastTime,"time decreases");
    const int local=count%block;
    for (int j=0;j<columns;++j) {
        const double v=row[j];
        ensure(std::isfinite(v),"non-finite sample");
        if (!local || v<minimum[j]) { minimum[j]=v; offsets[2*j]=local; }
        if (!local || v>maximum[j]) { maximum[j]=v; offsets[2*j+1]=local; }
    }
    if (local==block-1) index.insert(index.end(),offsets.begin(),offsets.end());
    ensure(file.write(reinterpret_cast<const char*>(row.data()),columns*8)==columns*8,"sample write failed");
    ++count;
    lastTime=row.front();
}
void WaveWriter::finish() {
    ensure(count>0,"empty waveform");
    if (count%block) index.insert(index.end(),offsets.begin(),offsets.end());
    const qint64 bytes=index.size()*sizeof(quint16);
    ensure(file.write(reinterpret_cast<const char*>(index.data()),bytes)==bytes &&
           file.seek(8) && file.write(reinterpret_cast<const char*>(&count),8)==8 &&
           file.commit(),"cannot commit wave file");
}
Waves::Waves(const QString& path):file(path) {
    ensure(file.open(QIODevice::ReadOnly),"cannot open wave file");
    auto bytes=file.read(header);
    ensure(bytes.size()==header && bytes.startsWith("LITZWV01"),"invalid wave header");
    std::memcpy(&rows,bytes.constData()+8,8);
    const int end=bytes.indexOf('\0',16);
    ensure(end>16,"invalid channel header");
    for (const auto value:QJsonDocument::fromJson(bytes.mid(16,end-16)).array()) channels<<value.toString();
    validateChannels(channels);
    const qint64 width=channels.size();
    ensure(rows>0 && rows<=(file.size()-header)/(8*width),"invalid wave length");
    const auto blocks=(rows+block-1)/block;
    const qint64 samples=rows*8*width;
    ensure(file.size()-header-samples==blocks*width*4,"incomplete wave data");
    const auto* mapped=file.map(0,file.size());
    ensure(mapped,"cannot map wave file");
    data=reinterpret_cast<const double*>(mapped+header);
    index=reinterpret_cast<const quint16*>(mapped+header+samples);
}
double Waves::at(qint64 row,int col) const {
    ensure(row>=0 && row<rows && col>=0 && col<channels.size(),"sample outside wave file");
    return data[row*channels.size()+col];
}
std::vector<QPointF> Waves::envelope(double from,double to,int y,int pixels) const {
    std::vector<QPointF> points;
    for(auto row:envelopeRows(from,to,y,pixels)) points.emplace_back(at(row,0),at(row,y));
    return points;
}
std::vector<qint64> Waves::envelopeRows(double from,double to,int y,int pixels) const {
    ensure(from<=to && pixels>0 && y>=0 && y<channels.size(),"invalid wave viewport");
    auto bound=[&](double time,bool upper) {
        qint64 lo=0,hi=rows;
        while (lo<hi) {
            auto mid=lo+(hi-lo)/2;
            if (upper ? at(mid,0)<=time : at(mid,0)<time) lo=mid+1; else hi=mid;
        }
        return lo;
    };
    const qint64 start=bound(from,false),end=bound(to,true);
    std::vector<qint64> points;
    const qint64 nominal=std::max<qint64>(1,(end-start+pixels-1)/pixels);
    const qint64 stride=nominal>=block ? ((nominal+block-1)/block)*block : nominal;
    for (qint64 begin=start;begin<end;) {
        const qint64 limit=std::min(end,(begin/stride+1)*stride);
        qint64 low=begin,high=begin;
        for (qint64 r=begin;r<limit;) {
            qint64 a=r,b=r;
            if (r%block==0 && limit-r>=block) {
                auto idx=(r/block*channels.size()+y)*2;
                a+=index[idx]; b+=index[idx+1];
                ensure(a<r+block && b<r+block,"invalid block index");
                r+=block;
            } else ++r;
            if (at(a,y)<at(low,y)) low=a;
            if (at(b,y)>at(high,y)) high=b;
        }
        qint64 selected[]={begin,std::min(low,high),std::max(low,high),limit-1};
        for (auto r:selected) {
            if (points.empty() || points.back()!=r) points.push_back(r);
        }
        begin=limit;
    }
    return points;
}
void importCsv(const QString& input,const QString& output) {
    ensure(QFileInfo(input)!=QFileInfo(output),"CSV and wave paths must differ");
    QFile file(input);
    ensure(file.open(QIODevice::ReadOnly),"cannot open CSV");
    QStringList names=QString::fromUtf8(file.readLine()).trimmed().split(',');
    for (auto& name:names) name=name.trimmed();
    WaveWriter writer(output,names);
    std::vector<double> row(names.size());
    while (!file.atEnd()) {
        QByteArray line=file.readLine();
        auto fields=line.trimmed().split(',');
        ensure(fields.size()==names.size(),"CSV sample width mismatch");
        for (int j=0;j<fields.size();++j) {
            auto token=fields[j].trimmed();
            auto end=token.constData()+token.size();
            auto parsed=std::from_chars(token.constData(),end,row[j]);
            ensure(parsed.ec==std::errc() && parsed.ptr==end,"invalid numeric CSV field");
        }
        writer.append(row);
    }
    ensure(file.error()==QFileDevice::NoError,"CSV read failed");
    writer.finish();
}
