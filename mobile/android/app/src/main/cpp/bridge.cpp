// SPDX-License-Identifier: GPL-2.0-or-later
#include <jni.h>
#include <cstring>
#include <string>
#include <cstdint>
#include <vector>

static std::string utf8(JNIEnv *env, jstring text) {
    jclass type = env->FindClass("java/lang/String");
    jmethodID method = env->GetMethodID(type, "getBytes", "(Ljava/lang/String;)[B");
    jstring encoding = env->NewStringUTF("UTF-8");
    auto bytes = (jbyteArray)env->CallObjectMethod(text, method, encoding);
    std::string result(env->GetArrayLength(bytes), 0);
    env->GetByteArrayRegion(bytes, 0, result.size(), reinterpret_cast<jbyte *>(result.data()));
    env->DeleteLocalRef(bytes); env->DeleteLocalRef(encoding); env->DeleteLocalRef(type);
    return result;
}

static jstring javaString(JNIEnv *env, const char *text) {
    jclass type = env->FindClass("java/lang/String");
    jmethodID constructor = env->GetMethodID(type, "<init>", "([BLjava/lang/String;)V");
    auto size = static_cast<jsize>(std::strlen(text));
    jbyteArray bytes = env->NewByteArray(size);
    env->SetByteArrayRegion(bytes, 0, size, reinterpret_cast<const jbyte *>(text));
    jstring encoding = env->NewStringUTF("UTF-8");
    auto result = (jstring)env->NewObject(type, constructor, bytes, encoding);
    env->DeleteLocalRef(bytes); env->DeleteLocalRef(encoding); env->DeleteLocalRef(type);
    return result;
}
extern "C" {
int rsxp_mobile_configure(const char *path,const char *runtime);
const unsigned char *rsxp_mobile_frame(uint32_t *width,uint32_t *height,size_t *length);
char *rsxp_mobile_request(const char *json);
void rsxp_mobile_free(char *result);
JNIEXPORT jboolean JNICALL Java_org_rsxp_mobile_MainActivity_nativeConfigure(JNIEnv *env, jclass, jstring path,jstring runtime) {
    const auto text = utf8(env, path);
    const auto ruby = utf8(env,runtime);
    const bool ok = rsxp_mobile_configure(text.c_str(),ruby.c_str()) == 1;
    return ok;
}
JNIEXPORT jstring JNICALL Java_org_rsxp_mobile_MainActivity_nativeRequest(JNIEnv *env, jclass, jstring json) {
    const auto text = utf8(env, json);
    char *response = rsxp_mobile_request(text.c_str());
    jstring result = javaString(env, response);
    rsxp_mobile_free(response);
    return result;
}
JNIEXPORT jintArray JNICALL Java_org_rsxp_mobile_MainActivity_nativeFrame(JNIEnv *env,jclass) {
    uint32_t width=0,height=0;size_t length=0;
    const auto rgba=rsxp_mobile_frame(&width,&height,&length);
    if(!rgba || !width || !height) return nullptr;
    std::vector<jint> frame(2+width*height);frame[0]=width;frame[1]=height;
    for(size_t i=0;i<width*height;i++) frame[2+i]=(jint)((uint32_t(rgba[4*i+3])<<24)|(uint32_t(rgba[4*i])<<16)|(uint32_t(rgba[4*i+1])<<8)|rgba[4*i+2]);
    auto result=env->NewIntArray(frame.size());env->SetIntArrayRegion(result,0,frame.size(),frame.data());return result;
}
}
