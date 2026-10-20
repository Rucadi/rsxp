// SPDX-License-Identifier: GPL-2.0-or-later
// All Ruby API calls and Ruby longjmp boundaries stay inside this C adapter.
#include <ruby.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

struct RsxpNativeResult { unsigned char *data; size_t length; int status; };
extern struct RsxpNativeResult rsxp_mobile_native_bytes(const unsigned char *, size_t);
extern void rsxp_mobile_native_free(unsigned char *, size_t);
#ifndef RSXP_STOCK_RUBY
extern void CRuby_init(bool, void (*)(void));
#endif
static char *runtime_path;
static char error_text[16384];
static bool initialized;
static VALUE game_fiber;

static VALUE dispatch(VALUE self, VALUE request) {
    (void)self;
    StringValue(request);
    struct RsxpNativeResult result=rsxp_mobile_native_bytes((const unsigned char *)RSTRING_PTR(request),(size_t)RSTRING_LEN(request));
    if(result.status==1) return Qnil;
    VALUE response=rb_str_new((const char *)result.data,(long)result.length);
    rsxp_mobile_native_free(result.data,result.length);
    if(result.status==2) rb_exc_raise(rb_exc_new_str(rb_eRuntimeError,response));
    return response;
}
static void before_prelude(void) {
    VALUE paths=rb_gv_get("$LOAD_PATH");
    const char *suffixes[]={"/lib/ruby/4.0.0","/lib/ruby/vendor_ruby","/lib/ruby/site_ruby","/lib/ruby/rbconfig",NULL};
    for(int i=0;suffixes[i];i++) {
        VALUE path=rb_str_new_cstr(runtime_path); rb_str_cat_cstr(path,suffixes[i]); rb_ary_unshift(paths,path);
    }
    rb_define_const(rb_cObject,"CRUBY_BUILD_SDK_AND_ARCH",rb_str_new_cstr(RSXP_RUBY_PLATFORM));
}
static VALUE exception_message(VALUE exception) { return rb_funcall(exception,rb_intern("full_message"),0); }
static int capture(int state) {
    if(!state) return 0;
    VALUE exception=rb_errinfo();
    int error_state=0;
    VALUE message=rb_protect(exception_message,exception,&error_state);
    snprintf(error_text,sizeof(error_text),"%s",error_state ? "Native Ruby exception" : StringValueCStr(message));
    rb_set_errinfo(Qnil);
    return -1;
}
const char *rsxp_mri_error(void) { return error_text; }
int rsxp_mri_initialize(const char *path) {
    if(initialized) return 0;
    runtime_path=strdup(path);
    // Game fibers call substantial Rust rendering code. MRI's small default
    // machine stack overflows even for the demo in an unoptimized Rust build.
    // Native fiber stacks reserve address space and commit pages as needed.
    setenv("RUBY_FIBER_MACHINE_STACK_SIZE", "8388608", 0);
#ifdef RSXP_STOCK_RUBY
    RUBY_INIT_STACK;
    ruby_init(); ruby_init_loadpath(); before_prelude();
    char *argv[]={"rsxp-mobile","--disable-gems","-e","",NULL};
    ruby_options(4,argv);
#else
    CRuby_init(false,before_prelude);
#endif
    rb_define_global_function("__rsxp_native",dispatch,1);
    rb_define_global_function("__mkxp_native",dispatch,1);
    game_fiber=Qnil; rb_global_variable(&game_fiber);
    initialized=true;
    return 0;
}
struct Start { const char *source,*root,*scripts,*script,*title,*saves; };
static VALUE start_game(VALUE argument) {
    struct Start *start=(struct Start *)argument;
    VALUE environment=rb_const_get(rb_cObject,rb_intern("ENV"));
    const char *keys[]={"RSXP_SCRIPTS","RSXP_SCRIPT","RSXP_GAME_TITLE","XDG_DATA_HOME"};
    const char *values[]={start->scripts,start->script,start->title,start->saves};
    for(int i=0;i<4;i++) rb_funcall(environment,rb_intern("[]="),2,rb_str_new_cstr(keys[i]),values[i] ? rb_str_new_cstr(values[i]) : Qnil);
    rb_funcall(rb_cDir,rb_intern("chdir"),1,rb_str_new_cstr(start->root));
    rb_gv_set("$rsxp_mobile_source",rb_utf8_str_new_cstr(start->source));
    rb_eval_string(
      "module RSXP;"
      " def self.startup_yield; @mobile_work=(@mobile_work||0)+1; Fiber.yield if @mobile_work%32==0; end;"
      " def self.before_scripts;"
      "  Graphics.singleton_class.class_eval do;"
      "   alias_method :__rsxp_mobile_update, :update;"
      "   def update; __rsxp_mobile_update; Fiber.yield; end;"
      "  end;"
      " end;"
      "end");
    game_fiber=rb_eval_string("Fiber.new { eval($rsxp_mobile_source,TOPLEVEL_BINDING,'rgss.rb',1) }");
    return Qnil;
}
int rsxp_mri_start(const char *source,const char *root,const char *scripts,const char *script,const char *title,const char *saves) {
    struct Start start={source,root,scripts,script,title,saves};
    int state=0; rb_protect(start_game,(VALUE)&start,&state); return capture(state);
}
static VALUE resume_game(VALUE unused) {
    (void)unused;
    if(NIL_P(game_fiber) || !RTEST(rb_funcall(game_fiber,rb_intern("alive?"),0))) return Qfalse;
    rb_funcall(game_fiber,rb_intern("resume"),0);
    return rb_funcall(game_fiber,rb_intern("alive?"),0);
}
int rsxp_mri_step(void) {
    int state=0; VALUE alive=rb_protect(resume_game,Qnil,&state);
    return capture(state) ? -1 : RTEST(alive) ? 0 : 1;
}
static VALUE stop_game(VALUE unused) { (void)unused; game_fiber=Qnil; rb_gc_start(); return Qnil; }
void rsxp_mri_stop(void) { if(initialized) { int state=0; rb_protect(stop_game,Qnil,&state); capture(state); } }
