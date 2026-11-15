#!/bin/sh
# Run inside unmodified ffmpeg-8.0.3 source. A neutral prefix keeps personal
# build paths out of avcodec_configuration() and the release artifact.
set -eu
export MACOSX_DEPLOYMENT_TARGET=14.0
./configure --prefix=/tmp/dorobot-rc-ffmpeg --enable-shared --disable-static --disable-doc --disable-debug --disable-autodetect --disable-everything --disable-ffplay --enable-pic --enable-avcodec --enable-avformat --enable-avutil --enable-avfilter --enable-avdevice --enable-indev=avfoundation --enable-swscale --enable-swresample --enable-protocol=file --enable-demuxer=mov,matroska,avi --enable-parser=h264,hevc,mpeg4video,mjpeg --enable-decoder=h264,hevc,mpeg4,mjpeg,rawvideo --enable-filter=null,anull,scale --enable-encoder=rawvideo,mpeg4 --enable-muxer=null,rawvideo,mp4
make -j8
make install
